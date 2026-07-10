use super::ConditionalArithParser;
use crate::executor::arithmetic::{
    assignment_operator_at, bash_arith, checked_arithmetic_pow, skip_arith_ws,
};
use crate::executor::{is_shell_name_char, is_shell_name_start};

impl ConditionalArithParser<'_> {
    pub(in crate::executor::arithmetic) fn parse_comma(&mut self) -> Option<i128> {
        let mut value = self.parse_assignment()?;
        loop {
            self.skip_ws();
            if !self.consume(",") {
                return Some(value);
            }
            value = self.parse_assignment()?;
        }
    }

    pub(super) fn parse_assignment(&mut self) -> Option<i128> {
        self.skip_ws();
        let start = self.pos;
        if self.assignment_lvalue_is_next() {
            self.pos = start;
            let lvalue = self.parse_lvalue()?;
            self.skip_ws();
            if let Some(op) = self.consume_assignment_operator() {
                let rhs = self.parse_assignment()?;
                return self.assign_lvalue(&lvalue, op, rhs);
            }
        }
        self.pos = start;
        self.parse_conditional()
    }

    pub(super) fn assignment_lvalue_is_next(&self) -> bool {
        let mut pos = self.pos;
        skip_arith_ws(self.input, &mut pos);
        let Some(first) = self.input.get(pos).copied().map(char::from) else {
            return false;
        };
        if !is_shell_name_start(first) {
            return false;
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
                return false;
            }
        }
        skip_arith_ws(self.input, &mut pos);
        assignment_operator_at(self.input, pos).is_some()
    }

    pub(super) fn parse_conditional(&mut self) -> Option<i128> {
        let condition = self.parse_logical_or()?;
        self.skip_ws();
        if !self.consume("?") {
            return Some(condition);
        }

        if condition == 0 {
            self.skip_arithmetic_conditional_branch(&[":"]);
            self.skip_ws();
            if !self.consume(":") {
                return None;
            }
            return self.parse_assignment();
        }

        let true_value = self.parse_comma()?;
        self.skip_ws();
        if !self.consume(":") {
            return None;
        }
        self.skip_arithmetic_conditional_branch(&[",", ")", ":"]);
        Some(true_value)
    }

    pub(super) fn parse_logical_or(&mut self) -> Option<i128> {
        let mut left = self.parse_logical_and()?;
        loop {
            self.skip_ws();
            if !self.consume("||") {
                return Some(left);
            }
            if left != 0 {
                self.skip_arithmetic_rhs(&["||", ",", "?", ":", ")"]);
                left = 1;
                continue;
            }
            let right = self.parse_logical_and()?;
            left = i128::from(left != 0 || right != 0);
        }
    }

    pub(super) fn parse_logical_and(&mut self) -> Option<i128> {
        let mut left = self.parse_bitwise_or()?;
        loop {
            self.skip_ws();
            if !self.consume("&&") {
                return Some(left);
            }
            if left == 0 {
                self.skip_arithmetic_rhs(&["&&", "||", ",", "?", ":", ")"]);
                continue;
            }
            let right = self.parse_bitwise_or()?;
            left = i128::from(left != 0 && right != 0);
        }
    }

    pub(super) fn parse_bitwise_or(&mut self) -> Option<i128> {
        let mut left = self.parse_bitwise_xor()?;
        loop {
            self.skip_ws();
            if self.starts_with("||") {
                return Some(left);
            }
            if self.consume("|") {
                left = bash_arith(left | self.parse_bitwise_xor()?);
            } else {
                return Some(left);
            }
        }
    }

    pub(super) fn parse_bitwise_xor(&mut self) -> Option<i128> {
        let mut left = self.parse_bitwise_and()?;
        loop {
            self.skip_ws();
            if self.consume("^") {
                left = bash_arith(left ^ self.parse_bitwise_and()?);
            } else {
                return Some(left);
            }
        }
    }

    pub(super) fn parse_bitwise_and(&mut self) -> Option<i128> {
        let mut left = self.parse_comparison()?;
        loop {
            self.skip_ws();
            if self.starts_with("&&") {
                return Some(left);
            }
            if self.consume("&") {
                left = bash_arith(left & self.parse_comparison()?);
            } else {
                return Some(left);
            }
        }
    }

    pub(super) fn parse_comparison(&mut self) -> Option<i128> {
        let mut left = self.parse_shift()?;
        loop {
            self.skip_ws();
            let result = if self.consume("==") {
                left == self.parse_shift()?
            } else if self.consume("!=") {
                left != self.parse_shift()?
            } else if self.consume(">=") {
                left >= self.parse_shift()?
            } else if self.consume("<=") {
                left <= self.parse_shift()?
            } else if self.consume(">") {
                left > self.parse_shift()?
            } else if self.consume("<") {
                left < self.parse_shift()?
            } else {
                return Some(left);
            };
            left = i128::from(result);
        }
    }

    pub(super) fn parse_shift(&mut self) -> Option<i128> {
        let mut value = self.parse_expr()?;
        loop {
            self.skip_ws();
            if self.consume("<<") {
                let rhs = self.parse_expr()?;
                let shift = u32::try_from(rhs).ok()?;
                value = bash_arith((value as i64).wrapping_shl(shift) as i128);
            } else if self.consume(">>") {
                let rhs = self.parse_expr()?;
                let shift = u32::try_from(rhs).ok()?;
                value = bash_arith((value as i64).wrapping_shr(shift) as i128);
            } else {
                return Some(value);
            }
        }
    }

    pub(super) fn parse_expr(&mut self) -> Option<i128> {
        let mut value = self.parse_term()?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'+') => {
                    self.pos += 1;
                    value = bash_arith(value + self.parse_term()?);
                }
                Some(b'-') => {
                    self.pos += 1;
                    value = bash_arith(value - self.parse_term()?);
                }
                _ => return Some(value),
            }
        }
    }

    pub(super) fn parse_term(&mut self) -> Option<i128> {
        let mut value = self.parse_power()?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'*') => {
                    if self.starts_with("**") {
                        return Some(value);
                    }
                    self.pos += 1;
                    value = bash_arith(value * self.parse_power()?);
                }
                Some(b'/') => {
                    self.pos += 1;
                    let rhs = self.parse_power()?;
                    if rhs == 0 {
                        return None;
                    }
                    value = bash_arith((value as i64).wrapping_div(rhs as i64) as i128);
                }
                Some(b'%') => {
                    self.pos += 1;
                    let rhs = self.parse_power()?;
                    if rhs == 0 {
                        return None;
                    }
                    if value == i128::from(i64::MIN) && rhs == -1 {
                        return None;
                    }
                    value %= rhs;
                }
                _ => return Some(value),
            }
        }
    }

    pub(super) fn parse_power(&mut self) -> Option<i128> {
        let value = self.parse_factor()?;
        self.skip_ws();
        if self.consume("**") {
            let rhs = self.parse_power()?;
            checked_arithmetic_pow(value, rhs)
        } else {
            Some(value)
        }
    }
}
