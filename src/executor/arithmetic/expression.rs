use super::ConditionalArithParser;
use crate::executor::arithmetic::{
    assignment_operator_at, bash_arith, checked_arithmetic_pow, skip_arith_ws,
};
use crate::executor::{is_shell_name_char, is_shell_name_start};

impl ConditionalArithParser<'_> {
    /// GNU expr.c:496-508 expcomma.
    pub(in crate::executor::arithmetic) fn parse_comma(&mut self) -> Option<i128> {
        let mut value = self.parse_assignment()?;
        loop {
            self.skip_ws();
            if self.peek() != Some(b',') {
                return Some(value);
            }
            self.note_op(self.pos);
            self.pos += 1;
            value = self.parse_assignment()?;
        }
    }

    /// GNU expr.c:511-626 expassign. A bare assignment target behind
    /// `&&`/`||` or a ternary branch is still a parse-level "attempted
    /// assignment to non-variable" (expr.c:528-529) — the assignment
    /// operator trails an operand position whose lasttok is not STR.
    pub(super) fn parse_assignment(&mut self) -> Option<i128> {
        self.skip_ws();
        let start = self.pos;
        if self.assignment_lvalue_is_next() {
            self.pos = start;
            // GNU expr.c:1395-1401: when the next token is `=`, the lvalue
            // subscript is NOT pre-evaluated. The raw token string is saved and
            // the subscript is re-evaluated at bind time (after the RHS),
            // so side effects in the RHS are visible to the subscript
            // (e.g. `a[n]=++n` stores at a[1], not a[0]).
            let lvalue = self.parse_lvalue_for_assignment()?;
            self.skip_ws();
            let op_start = self.pos;
            if let Some(op) = self.consume_assignment_operator() {
                self.note_op(op_start);
                self.skip_ws();
                let rhs_start = self.pos;
                let rhs = self.parse_assignment()?;
                // GNU expr.c:549-555: `a /= 0` / `a %= 0` evalerrors
                // "division by 0" with lasttp at the RHS operand — unless
                // the assignment sits in a noeval region, where the RHS
                // becomes 1 and the (suppressed) bind proceeds.
                if matches!(op, "/=" | "%=") && rhs == 0 && self.noeval == 0 {
                    return self.fail("division by 0", rhs_start);
                }
                return self.assign_lvalue(&lvalue, op, rhs);
            }
        }
        self.pos = start;
        let value = self.parse_conditional()?;
        self.skip_ws();
        if assignment_operator_at(self.input, self.pos).is_some() {
            // GNU expr.c:521-529: curtok is EQ/OP_ASSIGN but lasttok is not
            // STR — any preceding reduction (`x + y = 9`, `(x) = 5`,
            // `a ? b : c = 5`, `x++ = 7`, `9 = 8`) set lasttok to the
            // operator/NUM/RPAR/COND kind, so this is "attempted assignment
            // to non-variable" with lasttp at the operator. The failure
            // fires inside the RHS's own expassign call, so an outer
            // pending bind (`x = 9 = 8`) never happens.
            return self.fail("attempted assignment to non-variable", self.pos);
        }
        Some(value)
    }

    /// Byte offset of the assignment operator following an lvalue-shaped
    /// run of text, or None when the text is not `name[...]? OP=`. Used to
    /// point lasttp at the operator for "attempted assignment to
    /// non-variable" (GNU expr.c:528-529).
    pub(super) fn assignment_op_start(&self) -> Option<usize> {
        let mut pos = self.pos;
        skip_arith_ws(self.input, &mut pos);
        let first = self.input.get(pos).copied().map(char::from)?;
        if !is_shell_name_start(first) {
            return None;
        }
        pos += 1;
        while self
            .input
            .get(pos)
            .is_some_and(|ch| is_shell_name_char(*ch as char))
        {
            pos += 1;
        }
        skip_arith_ws(self.input, &mut pos);
        if self.input.get(pos) == Some(&b'[') {
            pos += 1;
            let mut depth = 1usize;
            while pos < self.input.len() {
                match self.input[pos] {
                    b'[' => depth += 1,
                    b']' => {
                        depth -= 1;
                        if depth == 0 {
                            pos += 1;
                            break;
                        }
                    }
                    _ => {}
                }
                pos += 1;
            }
            if depth != 0 {
                return None;
            }
        }
        skip_arith_ws(self.input, &mut pos);
        assignment_operator_at(self.input, pos).map(|_| pos)
    }

    pub(super) fn assignment_lvalue_is_next(&self) -> bool {
        self.assignment_op_start().is_some()
    }

    /// GNU expr.c:630-674 expcond. The `?` branch is parsed with
    /// EXP_LOWEST (commas allowed); the `:` branch is parsed with expcond
    /// only, so a trailing assignment operator there is caught by the outer
    /// expassign with lasttok==COND -> "attempted assignment to
    /// non-variable" (`1 ? 20 : x+=2` reports `+=2`).
    pub(super) fn parse_conditional(&mut self) -> Option<i128> {
        let condition = self.parse_logical_or()?;
        self.skip_ws();
        if self.peek() != Some(b'?') {
            return Some(condition);
        }
        self.note_op(self.pos);
        self.pos += 1;
        self.skip_ws();
        // GNU expr.c:645-647: `?` followed by `:` or end-of-input ->
        // "expression expected", checked before the true branch regardless
        // of the condition value.
        if self.pos >= self.input.len() || self.peek() == Some(b':') {
            let tok = if self.pos < self.input.len() {
                self.pos
            } else {
                self.last_tok_start
            };
            return self.fail("expression expected", tok);
        }

        if condition == 0 {
            // GNU expr.c:645-655: the true branch parses under noeval
            // (EXP_LOWEST — assignments are legal there, just unbound) —
            // so `0?3+:2` still errors "operand expected" with lasttp at
            // the token that failed to start an operand (`:2`).
            self.noeval += 1;
            let branch = self.parse_comma();
            self.noeval -= 1;
            branch?;
            self.skip_ws();
            // GNU expr.c:653-654: no `:` -> "`:' expected for conditional
            // expression" with lasttp at the current token.
            if self.peek() != Some(b':') {
                let tok = if self.pos < self.input.len() {
                    self.pos
                } else {
                    self.last_tok_start
                };
                return self.fail("`:' expected for conditional expression", tok);
            }
            self.note_op(self.pos);
            self.pos += 1;
            self.skip_ws();
            // GNU expr.c:663-665: `:` at end-of-input -> "expression
            // expected" (token is the `:`).
            if self.pos >= self.input.len() {
                return self.fail("expression expected", self.last_tok_start);
            }
            return self.parse_conditional();
        }

        let true_value = self.parse_comma()?;
        self.skip_ws();
        if self.peek() != Some(b':') {
            let tok = if self.pos < self.input.len() {
                self.pos
            } else {
                self.last_tok_start
            };
            return self.fail("`:' expected for conditional expression", tok);
        }
        self.note_op(self.pos);
        self.pos += 1;
        self.skip_ws();
        if self.pos >= self.input.len() {
            return self.fail("expression expected", self.last_tok_start);
        }
        if let Some(op_pos) = self.assignment_op_start() {
            // GNU expr.c:528-529 via 666: the false branch is expcond, so an
            // assignment operator trails it -> non-variable (token = the op).
            self.skip_arithmetic_conditional_branch(&[",", ")", ":"]);
            return self.fail("attempted assignment to non-variable", op_pos);
        }
        // GNU expr.c:655-668: the false branch parses under noeval with
        // `expcond` — malformed text (`1?2:3+`) errors "operand expected"
        // while reads/binds are suppressed.
        self.noeval += 1;
        let branch = self.parse_conditional();
        self.noeval -= 1;
        branch?;
        Some(true_value)
    }

    /// GNU expr.c:678-702 explor. The RHS is parsed unconditionally under
    /// `noeval` (expr.c:690-691) — GNU never skips the text, it only
    /// suppresses reads/binds — so a malformed RHS (`7||`, `7||@`) still
    /// evalerrors "operand expected" with lasttp at the operator.
    pub(super) fn parse_logical_or(&mut self) -> Option<i128> {
        let mut left = self.parse_logical_and()?;
        loop {
            self.skip_ws();
            if !self.starts_with("||") {
                return Some(left);
            }
            self.note_op(self.pos);
            self.pos += 2;
            self.skip_ws();
            if let Some(op_pos) = self.assignment_op_start() {
                // GNU names this case explicitly: assignment targets must be
                // variables even behind || evaluation (expr.c:528-529, the
                // `=` trails the `||` operand -> lasttok is LOR, not STR).
                return self.fail("attempted assignment to non-variable", op_pos);
            }
            if left != 0 {
                self.noeval += 1;
                let right = self.parse_logical_and();
                self.noeval -= 1;
                right?;
                left = 1;
            } else {
                let right = self.parse_logical_and()?;
                left = i128::from(left != 0 || right != 0);
            }
        }
    }

    /// GNU expr.c:705-729 expland. Same noeval parse of the short-circuited
    /// RHS (expr.c:717-718): `0&&` errors "operand expected" with lasttp at
    /// `&&`.
    pub(super) fn parse_logical_and(&mut self) -> Option<i128> {
        let mut left = self.parse_bitwise_or()?;
        loop {
            self.skip_ws();
            if !self.starts_with("&&") {
                return Some(left);
            }
            self.note_op(self.pos);
            self.pos += 2;
            self.skip_ws();
            if let Some(op_pos) = self.assignment_op_start() {
                return self.fail("attempted assignment to non-variable", op_pos);
            }
            if left == 0 {
                self.noeval += 1;
                let right = self.parse_bitwise_or();
                self.noeval -= 1;
                right?;
            } else {
                let right = self.parse_bitwise_or()?;
                left = i128::from(left != 0 && right != 0);
            }
        }
    }

    /// GNU expr.c:1497-1500: `x=` where `x` is one of `*/%+-&^|` (or the
    /// `<<`/`>>`/`**` compounds) tokenizes as a single OP_ASSIGN — never as
    /// a binary operator followed by `=`. Binary-op loops must therefore
    /// leave `op=` text alone so expassign can report "attempted assignment
    /// to non-variable" with lasttp at the whole operator (`x + y += 9`
    /// reports `+= 9 `, not an operand-expected `=`).
    fn at_assignment_op(&self) -> bool {
        assignment_operator_at(self.input, self.pos).is_some()
    }

    /// GNU expr.c:732-746 expbitor.
    pub(super) fn parse_bitwise_or(&mut self) -> Option<i128> {
        let mut left = self.parse_bitwise_xor()?;
        loop {
            self.skip_ws();
            if self.starts_with("||") || self.at_assignment_op() {
                return Some(left);
            }
            if self.peek() == Some(b'|') {
                self.note_op(self.pos);
                self.pos += 1;
                left = bash_arith(left | self.parse_bitwise_xor()?);
            } else {
                return Some(left);
            }
        }
    }

    /// GNU expr.c:749-763 expbitxor.
    pub(super) fn parse_bitwise_xor(&mut self) -> Option<i128> {
        let mut left = self.parse_bitwise_and()?;
        loop {
            self.skip_ws();
            if self.at_assignment_op() {
                return Some(left);
            }
            if self.peek() == Some(b'^') {
                self.note_op(self.pos);
                self.pos += 1;
                left = bash_arith(left ^ self.parse_bitwise_and()?);
            } else {
                return Some(left);
            }
        }
    }

    /// GNU expr.c:766-780 expbitand.
    pub(super) fn parse_bitwise_and(&mut self) -> Option<i128> {
        let mut left = self.parse_comparison()?;
        loop {
            self.skip_ws();
            if self.starts_with("&&") || self.at_assignment_op() {
                return Some(left);
            }
            if self.peek() == Some(b'&') {
                self.note_op(self.pos);
                self.pos += 1;
                left = bash_arith(left & self.parse_comparison()?);
            } else {
                return Some(left);
            }
        }
    }

    /// GNU expr.c:783-831 expcompar. The operator token is noted as it is
    /// consumed — like every other precedence level — so a failing RHS
    /// parse reports `lasttp` at the comparison operator itself
    /// (`7<=` at end of input -> "operand expected", token `<=`,
    /// readtok set lasttp when it read the operator, expr.c:1342).
    pub(super) fn parse_comparison(&mut self) -> Option<i128> {
        let mut left = self.parse_shift()?;
        loop {
            self.skip_ws();
            let op_start = self.pos;
            let result = if self.starts_with("==") {
                self.note_op(op_start);
                self.pos += 2;
                Some(left == self.parse_shift()?)
            } else if self.starts_with("!=") {
                self.note_op(op_start);
                self.pos += 2;
                Some(left != self.parse_shift()?)
            } else if self.starts_with(">=") {
                self.note_op(op_start);
                self.pos += 2;
                Some(left >= self.parse_shift()?)
            } else if self.starts_with("<=") {
                self.note_op(op_start);
                self.pos += 2;
                Some(left <= self.parse_shift()?)
            } else if self.peek() == Some(b'>') {
                self.note_op(op_start);
                self.pos += 1;
                Some(left > self.parse_shift()?)
            } else if self.peek() == Some(b'<') {
                self.note_op(op_start);
                self.pos += 1;
                Some(left < self.parse_shift()?)
            } else {
                None
            };
            let Some(result) = result else {
                return Some(left);
            };
            left = i128::from(result);
        }
    }

    /// GNU expr.c:834-848 expshift.
    pub(super) fn parse_shift(&mut self) -> Option<i128> {
        let mut value = self.parse_expr()?;
        loop {
            self.skip_ws();
            if self.at_assignment_op() {
                return Some(value);
            }
            if self.starts_with("<<") {
                self.note_op(self.pos);
                self.pos += 2;
                let rhs = self.parse_expr()?;
                let shift = u32::try_from(rhs).ok()?;
                value = bash_arith((value as i64).wrapping_shl(shift) as i128);
            } else if self.starts_with(">>") {
                self.note_op(self.pos);
                self.pos += 2;
                let rhs = self.parse_expr()?;
                let shift = u32::try_from(rhs).ok()?;
                value = bash_arith((value as i64).wrapping_shr(shift) as i128);
            } else {
                return Some(value);
            }
        }
    }

    /// GNU expr.c:851-865 expaddsub.
    pub(super) fn parse_expr(&mut self) -> Option<i128> {
        let mut value = self.parse_term()?;
        loop {
            self.skip_ws();
            if self.at_assignment_op() {
                return Some(value);
            }
            match self.peek() {
                Some(b'+') => {
                    self.note_op(self.pos);
                    self.pos += 1;
                    value = bash_arith(value + self.parse_term()?);
                }
                Some(b'-') => {
                    self.note_op(self.pos);
                    self.pos += 1;
                    value = bash_arith(value - self.parse_term()?);
                }
                _ => return Some(value),
            }
        }
    }

    /// GNU expr.c:868-937 expmuldiv — "division by 0" evalerrors with
    /// lasttp at the RHS operand start (unary sign included, expr.c:911).
    pub(super) fn parse_term(&mut self) -> Option<i128> {
        let mut value = self.parse_power()?;
        loop {
            self.skip_ws();
            if self.at_assignment_op() {
                return Some(value);
            }
            match self.peek() {
                Some(b'*') => {
                    if self.starts_with("**") {
                        return Some(value);
                    }
                    self.note_op(self.pos);
                    self.pos += 1;
                    value = bash_arith(value * self.parse_power()?);
                }
                Some(b'/') => {
                    self.note_op(self.pos);
                    self.pos += 1;
                    self.skip_ws();
                    let rhs_start = self.pos;
                    let mut rhs = self.parse_power()?;
                    if rhs == 0 {
                        // GNU expr.c:909-920: under noeval the divisor
                        // becomes 1 instead of evalerroring.
                        if self.noeval == 0 {
                            return self.fail("division by 0", rhs_start);
                        }
                        rhs = 1;
                    }
                    value = bash_arith((value as i64).wrapping_div(rhs as i64) as i128);
                }
                Some(b'%') => {
                    self.note_op(self.pos);
                    self.pos += 1;
                    self.skip_ws();
                    let rhs_start = self.pos;
                    let mut rhs = self.parse_power()?;
                    if rhs == 0 {
                        // GNU expr.c:909-920: noeval suppresses the
                        // evalerror and divides by 1 instead.
                        if self.noeval == 0 {
                            return self.fail("division by 0", rhs_start);
                        }
                        rhs = 1;
                    }
                    // GNU expr.c:923-926: INTMAX_MIN % -1 is 0 (avoids
                    // SIGFPE from undefined behavior on x86).
                    if value == i128::from(i64::MIN) && rhs == -1 {
                        value = 0;
                    } else {
                        value %= rhs;
                    }
                }
                _ => return Some(value),
            }
        }
    }

    /// GNU expr.c:940-1000 exppower — a negative exponent evalerrors
    /// "exponent less than 0" with lasttp at the exponent's NUM token
    /// (`2 ** -1` reports `1`, not `-1`).
    pub(super) fn parse_power(&mut self) -> Option<i128> {
        let value = self.parse_factor()?;
        self.skip_ws();
        if self.starts_with("**") && !self.at_assignment_op() {
            self.note_op(self.pos);
            self.pos += 2;
            self.skip_ws();
            // GNU lasttp is the first digit of the exponent: unary `-`
            // is its own token.
            let mut tok = self.pos;
            if matches!(self.input.get(tok), Some(b'-') | Some(b'+')) {
                tok += 1;
                skip_arith_ws(self.input, &mut tok);
            }
            let rhs = self.parse_power()?;
            if rhs < 0 {
                // GNU expr.c:977-991: under noeval the negative-exponent
                // evalerror is suppressed and the result is 1.
                if self.noeval == 0 {
                    return self.fail("exponent less than 0", tok);
                }
                return Some(1);
            }
            checked_arithmetic_pow(value, rhs)
        } else {
            Some(value)
        }
    }
}
