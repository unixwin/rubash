use super::factor::parse_factor_with;
use super::{ArithBackend, ArithBinOp, ConditionalArithParser};
use crate::executor::arithmetic::{assignment_operator_at, skip_arith_ws};

impl ConditionalArithParser<'_> {
    /// GNU expr.c:496-508 expcomma.
    pub(in crate::executor::arithmetic) fn parse_comma(&mut self) -> Option<i128> {
        parse_comma_with(self)
    }

    /// Byte offset of the assignment operator following an lvalue-shaped
    /// run of text, or None when the text is not `name[...]? OP=`. Used to
    /// point lasttp at the operator for "attempted assignment to
    /// non-variable" (GNU expr.c:528-529).
    pub(super) fn assignment_op_start(&self) -> Option<usize> {
        assignment_op_start_in(self.input, self.pos)
    }

    pub(super) fn assignment_lvalue_is_next(&self) -> bool {
        self.assignment_op_start().is_some()
    }
}

/// Byte offset of the assignment operator following an lvalue-shaped
/// run of text starting at `start_pos`.
pub(super) fn assignment_op_start_in(input: &[u8], start_pos: usize) -> Option<usize> {
    let mut pos = start_pos;
    skip_arith_ws(input, &mut pos);
    let first = input.get(pos).copied()?;
    if !(first == b'_' || first.is_ascii_alphabetic()) {
        return None;
    }
    pos += 1;
    while input
        .get(pos)
        .is_some_and(|&ch| ch == b'_' || ch.is_ascii_alphanumeric())
    {
        pos += 1;
    }
    skip_arith_ws(input, &mut pos);
    if input.get(pos) == Some(&b'[') {
        pos += 1;
        let mut depth = 1usize;
        while pos < input.len() {
            match input[pos] {
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
    skip_arith_ws(input, &mut pos);
    assignment_operator_at(input, pos).map(|_| pos)
}

/// GNU expr.c:496-508 expcomma.
pub(super) fn parse_comma_with<B: ArithBackend + ?Sized>(b: &mut B) -> Option<B::Val> {
    if !b.enter_depth() {
        return None;
    }
    let result = (|| {
        let mut value = parse_assignment_with(b)?;
        loop {
            b.skip_ws();
            if b.peek() != Some(b',') {
                return Some(value);
            }
            b.note_op(b.pos());
            b.set_pos(b.pos() + 1);
            let rhs = parse_assignment_with(b)?;
            value = b.comma(value, rhs)?;
        }
    })();
    b.leave_depth();
    result
}

/// GNU expr.c:511-626 expassign.
pub(super) fn parse_assignment_with<B: ArithBackend + ?Sized>(b: &mut B) -> Option<B::Val> {
    b.skip_ws();
    let start = b.pos();
    if let Some(assigned) = b.try_parse_assignment()? {
        return Some(assigned);
    }
    b.set_pos(start);
    let value = parse_conditional_with(b)?;
    b.skip_ws();
    if assignment_operator_at(b.input(), b.pos()).is_some() {
        return b.fail("attempted assignment to non-variable", b.pos());
    }
    Some(value)
}

/// GNU expr.c:630-674 expcond.
pub(super) fn parse_conditional_with<B: ArithBackend + ?Sized>(b: &mut B) -> Option<B::Val> {
    let condition = parse_logical_or_with(b)?;
    b.skip_ws();
    if b.peek() != Some(b'?') {
        return Some(condition);
    }
    b.note_op(b.pos());
    b.set_pos(b.pos() + 1);
    b.skip_ws();
    if b.pos() >= b.input().len() || b.peek() == Some(b':') {
        let tok = if b.pos() < b.input().len() {
            b.pos()
        } else {
            b.last_tok_start()
        };
        return b.fail("expression expected", tok);
    }
    b.eval_conditional(condition)
}

/// GNU expr.c:678-702 explor.
pub(super) fn parse_logical_or_with<B: ArithBackend + ?Sized>(b: &mut B) -> Option<B::Val> {
    let mut left = parse_logical_and_with(b)?;
    loop {
        b.skip_ws();
        if !b.starts_with("||") {
            return Some(left);
        }
        b.note_op(b.pos());
        b.set_pos(b.pos() + 2);
        b.skip_ws();
        if let Some(op_pos) = assignment_op_start_in(b.input(), b.pos()) {
            return b.fail("attempted assignment to non-variable", op_pos);
        }
        left = b.eval_logical_or(left)?;
    }
}

/// GNU expr.c:705-729 expland.
pub(super) fn parse_logical_and_with<B: ArithBackend + ?Sized>(b: &mut B) -> Option<B::Val> {
    let mut left = parse_bitwise_or_with(b)?;
    loop {
        b.skip_ws();
        if !b.starts_with("&&") {
            return Some(left);
        }
        b.note_op(b.pos());
        b.set_pos(b.pos() + 2);
        b.skip_ws();
        if let Some(op_pos) = assignment_op_start_in(b.input(), b.pos()) {
            return b.fail("attempted assignment to non-variable", op_pos);
        }
        left = b.eval_logical_and(left)?;
    }
}

/// Alias used by `eval_logical_and` in backends to parse the RHS of `&&`.
pub(super) fn parse_conditional_tail_with<B: ArithBackend + ?Sized>(b: &mut B) -> Option<B::Val> {
    parse_bitwise_or_with(b)
}

/// GNU expr.c:732-746 expbitor.
fn parse_bitwise_or_with<B: ArithBackend + ?Sized>(b: &mut B) -> Option<B::Val> {
    let mut left = parse_bitwise_xor_with(b)?;
    loop {
        b.skip_ws();
        if b.starts_with("||") || assignment_operator_at(b.input(), b.pos()).is_some() {
            return Some(left);
        }
        if b.peek() == Some(b'|') {
            b.note_op(b.pos());
            b.set_pos(b.pos() + 1);
            let rhs = parse_bitwise_xor_with(b)?;
            left = b.binary(ArithBinOp::BitOr, left, rhs)?;
        } else {
            return Some(left);
        }
    }
}

/// GNU expr.c:749-763 expbitxor.
fn parse_bitwise_xor_with<B: ArithBackend + ?Sized>(b: &mut B) -> Option<B::Val> {
    let mut left = parse_bitwise_and_with(b)?;
    loop {
        b.skip_ws();
        if assignment_operator_at(b.input(), b.pos()).is_some() {
            return Some(left);
        }
        if b.peek() == Some(b'^') {
            b.note_op(b.pos());
            b.set_pos(b.pos() + 1);
            let rhs = parse_bitwise_and_with(b)?;
            left = b.binary(ArithBinOp::BitXor, left, rhs)?;
        } else {
            return Some(left);
        }
    }
}

/// GNU expr.c:766-780 expbitand.
fn parse_bitwise_and_with<B: ArithBackend + ?Sized>(b: &mut B) -> Option<B::Val> {
    let mut left = parse_comparison_with(b)?;
    loop {
        b.skip_ws();
        if b.starts_with("&&") || assignment_operator_at(b.input(), b.pos()).is_some() {
            return Some(left);
        }
        if b.peek() == Some(b'&') {
            b.note_op(b.pos());
            b.set_pos(b.pos() + 1);
            let rhs = parse_comparison_with(b)?;
            left = b.binary(ArithBinOp::BitAnd, left, rhs)?;
        } else {
            return Some(left);
        }
    }
}

/// GNU expr.c:783-831 expcompar.
fn parse_comparison_with<B: ArithBackend + ?Sized>(b: &mut B) -> Option<B::Val> {
    let mut left = parse_shift_with(b)?;
    loop {
        b.skip_ws();
        let op_start = b.pos();
        let (op, len) = if b.starts_with("==") {
            (ArithBinOp::Eq, 2)
        } else if b.starts_with("!=") {
            (ArithBinOp::Ne, 2)
        } else if b.starts_with(">=") {
            (ArithBinOp::Ge, 2)
        } else if b.starts_with("<=") {
            (ArithBinOp::Le, 2)
        } else if b.peek() == Some(b'>') {
            (ArithBinOp::Gt, 1)
        } else if b.peek() == Some(b'<') {
            (ArithBinOp::Lt, 1)
        } else {
            return Some(left);
        };
        b.note_op(op_start);
        b.set_pos(op_start + len);
        let rhs = parse_shift_with(b)?;
        left = b.binary(op, left, rhs)?;
    }
}

/// GNU expr.c:834-848 expshift.
fn parse_shift_with<B: ArithBackend + ?Sized>(b: &mut B) -> Option<B::Val> {
    let mut value = parse_expr_with(b)?;
    loop {
        b.skip_ws();
        if assignment_operator_at(b.input(), b.pos()).is_some() {
            return Some(value);
        }
        if b.starts_with("<<") {
            b.note_op(b.pos());
            b.set_pos(b.pos() + 2);
            let rhs = parse_expr_with(b)?;
            value = b.binary(ArithBinOp::Shl, value, rhs)?;
        } else if b.starts_with(">>") {
            b.note_op(b.pos());
            b.set_pos(b.pos() + 2);
            let rhs = parse_expr_with(b)?;
            value = b.binary(ArithBinOp::Shr, value, rhs)?;
        } else {
            return Some(value);
        }
    }
}

/// GNU expr.c:851-865 expaddsub.
fn parse_expr_with<B: ArithBackend + ?Sized>(b: &mut B) -> Option<B::Val> {
    let mut value = parse_term_with(b)?;
    loop {
        b.skip_ws();
        if assignment_operator_at(b.input(), b.pos()).is_some() {
            return Some(value);
        }
        match b.peek() {
            Some(b'+') => {
                b.note_op(b.pos());
                b.set_pos(b.pos() + 1);
                let rhs = parse_term_with(b)?;
                value = b.binary(ArithBinOp::Add, value, rhs)?;
            }
            Some(b'-') => {
                b.note_op(b.pos());
                b.set_pos(b.pos() + 1);
                let rhs = parse_term_with(b)?;
                value = b.binary(ArithBinOp::Sub, value, rhs)?;
            }
            _ => return Some(value),
        }
    }
}

/// GNU expr.c:868-937 expmuldiv.
fn parse_term_with<B: ArithBackend + ?Sized>(b: &mut B) -> Option<B::Val> {
    let mut value = parse_power_with(b)?;
    loop {
        b.skip_ws();
        if assignment_operator_at(b.input(), b.pos()).is_some() {
            return Some(value);
        }
        match b.peek() {
            Some(b'*') => {
                if b.starts_with("**") {
                    return Some(value);
                }
                b.note_op(b.pos());
                b.set_pos(b.pos() + 1);
                let rhs = parse_power_with(b)?;
                value = b.binary(ArithBinOp::Mul, value, rhs)?;
            }
            Some(b'/') => {
                b.note_op(b.pos());
                b.set_pos(b.pos() + 1);
                b.skip_ws();
                let rhs_start = b.pos();
                let rhs = parse_power_with(b)?;
                value = b.div(value, rhs, rhs_start)?;
            }
            Some(b'%') => {
                b.note_op(b.pos());
                b.set_pos(b.pos() + 1);
                b.skip_ws();
                let rhs_start = b.pos();
                let rhs = parse_power_with(b)?;
                value = b.rem(value, rhs, rhs_start)?;
            }
            _ => return Some(value),
        }
    }
}

/// GNU expr.c:940-1000 exppower.
fn parse_power_with<B: ArithBackend + ?Sized>(b: &mut B) -> Option<B::Val> {
    let value = parse_factor_with(b)?;
    b.skip_ws();
    if b.starts_with("**") && assignment_operator_at(b.input(), b.pos()).is_none() {
        b.note_op(b.pos());
        b.set_pos(b.pos() + 2);
        b.skip_ws();
        let mut tok = b.pos();
        if matches!(b.input().get(tok), Some(b'-') | Some(b'+')) {
            tok += 1;
            skip_arith_ws(b.input(), &mut tok);
        }
        let rhs = parse_power_with(b)?;
        b.pow(value, rhs, tok)
    } else {
        Some(value)
    }
}
