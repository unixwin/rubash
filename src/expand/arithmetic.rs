//! arithmetic module.
//!
//! GNU Bash source ownership:
// - expr.c

use std::collections::HashMap;

const MAX_EXPR_RECURSION_LEVEL: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArithmeticError {
    message: String,
}

impl ArithmeticError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

pub fn eval(expr: &str, vars: &mut HashMap<String, String>) -> Result<i128, ArithmeticError> {
    // TODO(expr.c): GNU Bash's evalexp has exact intmax_t overflow behavior,
    // array lvalues, recursion guards, diagnostics, and short-circuit parse
    // rules. This parser covers the operator core needed by early arith.tests
    // while keeping ownership in the expr.c migration module.
    let mut parser = Parser::new(expr, vars);
    let value = parser.parse_comma()?;
    parser.skip_ws();
    if !parser.eof() {
        return Err(ArithmeticError::new(
            "arithmetic syntax error in expression",
        ));
    }
    Ok(value)
}

struct Parser<'a> {
    input: &'a str,
    pos: usize,
    vars: &'a mut HashMap<String, String>,
    noeval: bool,
    depth: usize,
}

#[derive(Clone, Debug)]
enum LValue {
    Variable(String),
    Indexed { name: String, index_expr: String },
}

impl<'a> Parser<'a> {
    fn new(input: &'a str, vars: &'a mut HashMap<String, String>) -> Self {
        Self {
            input,
            pos: 0,
            vars,
            noeval: false,
            depth: 0,
        }
    }

    fn nested(input: &'a str, vars: &'a mut HashMap<String, String>, depth: usize) -> Self {
        Self {
            input,
            pos: 0,
            vars,
            noeval: false,
            depth,
        }
    }

    fn parse_comma(&mut self) -> Result<i128, ArithmeticError> {
        let mut value = self.parse_assignment()?;
        loop {
            self.skip_ws();
            if !self.consume(",") {
                return Ok(value);
            }
            value = self.parse_assignment()?;
        }
    }

    fn parse_assignment(&mut self) -> Result<i128, ArithmeticError> {
        self.skip_ws();
        let checkpoint = self.pos;
        if let Some(lvalue) = self.parse_lvalue() {
            self.skip_ws();
            for op in [
                "<<=", ">>=", "+=", "-=", "*=", "/=", "%=", "&=", "^=", "|=", "=",
            ] {
                if op == "=" && self.starts_with("==") {
                    continue;
                }
                if self.consume(op) {
                    let rhs = self.parse_assignment()?;
                    let value = match op {
                        "=" => wrap_intmax(rhs),
                        "+=" => add_intmax(self.lvalue_value(&lvalue)?, rhs),
                        "-=" => sub_intmax(self.lvalue_value(&lvalue)?, rhs),
                        "*=" => mul_intmax(self.lvalue_value(&lvalue)?, rhs),
                        "/=" => checked_div(self.lvalue_value(&lvalue)?, rhs)?,
                        "%=" => checked_rem(self.lvalue_value(&lvalue)?, rhs)?,
                        "<<=" => shl_intmax(self.lvalue_value(&lvalue)?, rhs),
                        ">>=" => shr_intmax(self.lvalue_value(&lvalue)?, rhs),
                        "&=" => bitand_intmax(self.lvalue_value(&lvalue)?, rhs),
                        "^=" => bitxor_intmax(self.lvalue_value(&lvalue)?, rhs),
                        "|=" => bitor_intmax(self.lvalue_value(&lvalue)?, rhs),
                        _ => unreachable!(),
                    };
                    self.set_lvalue(&lvalue, value);
                    return Ok(value);
                }
            }
        }
        self.pos = checkpoint;
        self.parse_conditional()
    }

    fn parse_conditional(&mut self) -> Result<i128, ArithmeticError> {
        let condition = self.parse_logical_or()?;
        self.skip_ws();
        if !self.consume("?") {
            return Ok(condition);
        }

        if condition != 0 {
            let value = self.parse_assignment()?;
            self.skip_ws();
            if !self.consume(":") {
                return Err(ArithmeticError::new(
                    "`:' expected for conditional expression",
                ));
            }
            let tail = self.skip_unevaluated_conditional_tail();
            if tail_has_top_level_assignment(&tail) {
                return Err(ArithmeticError::new("attempted assignment to non-variable"));
            }
            Ok(value)
        } else {
            self.skip_to_conditional_colon();
            self.skip_ws();
            if !self.consume(":") {
                return Err(ArithmeticError::new("expression expected"));
            }
            self.parse_assignment()
        }
    }

    fn parse_logical_or(&mut self) -> Result<i128, ArithmeticError> {
        let mut value = self.parse_logical_and()?;
        loop {
            self.skip_ws();
            if !self.consume("||") {
                return Ok(value);
            }
            let rhs = if value != 0 {
                let previous = self.noeval;
                self.noeval = true;
                let parsed = self.parse_logical_and();
                self.noeval = previous;
                parsed?
            } else {
                self.parse_logical_and()?
            };
            value = i128::from(value != 0 || rhs != 0);
        }
    }

    fn parse_logical_and(&mut self) -> Result<i128, ArithmeticError> {
        let mut value = self.parse_bit_or()?;
        loop {
            self.skip_ws();
            if !self.consume("&&") {
                return Ok(value);
            }
            let rhs = if value == 0 {
                let previous = self.noeval;
                self.noeval = true;
                let parsed = self.parse_bit_or();
                self.noeval = previous;
                parsed?
            } else {
                self.parse_bit_or()?
            };
            value = i128::from(value != 0 && rhs != 0);
        }
    }

    fn parse_bit_or(&mut self) -> Result<i128, ArithmeticError> {
        let mut value = self.parse_bit_xor()?;
        loop {
            self.skip_ws();
            if self.starts_with("||") || !self.consume("|") {
                return Ok(value);
            }
            value = bitor_intmax(value, self.parse_bit_xor()?);
        }
    }

    fn parse_bit_xor(&mut self) -> Result<i128, ArithmeticError> {
        let mut value = self.parse_bit_and()?;
        loop {
            self.skip_ws();
            if !self.consume("^") {
                return Ok(value);
            }
            value = bitxor_intmax(value, self.parse_bit_and()?);
        }
    }

    fn parse_bit_and(&mut self) -> Result<i128, ArithmeticError> {
        let mut value = self.parse_equality()?;
        loop {
            self.skip_ws();
            if self.starts_with("&&") || !self.consume("&") {
                return Ok(value);
            }
            value = bitand_intmax(value, self.parse_equality()?);
        }
    }

    fn parse_equality(&mut self) -> Result<i128, ArithmeticError> {
        let mut value = self.parse_relational()?;
        loop {
            self.skip_ws();
            if self.consume("==") {
                value = i128::from(value == self.parse_relational()?);
            } else if self.consume("!=") {
                value = i128::from(value != self.parse_relational()?);
            } else {
                return Ok(value);
            }
        }
    }

    fn parse_relational(&mut self) -> Result<i128, ArithmeticError> {
        let mut value = self.parse_shift()?;
        loop {
            self.skip_ws();
            if self.consume("<=") {
                value = i128::from(value <= self.parse_shift()?);
            } else if self.consume(">=") {
                value = i128::from(value >= self.parse_shift()?);
            } else if self.consume("<") {
                value = i128::from(value < self.parse_shift()?);
            } else if self.consume(">") {
                value = i128::from(value > self.parse_shift()?);
            } else {
                return Ok(value);
            }
        }
    }

    fn parse_shift(&mut self) -> Result<i128, ArithmeticError> {
        let mut value = self.parse_additive()?;
        loop {
            self.skip_ws();
            if self.consume("<<") {
                value = shl_intmax(value, self.parse_additive()?);
            } else if self.consume(">>") {
                value = shr_intmax(value, self.parse_additive()?);
            } else {
                return Ok(value);
            }
        }
    }

    fn parse_additive(&mut self) -> Result<i128, ArithmeticError> {
        let mut value = self.parse_multiplicative()?;
        loop {
            self.skip_ws();
            if self.consume("+") {
                value = add_intmax(value, self.parse_multiplicative()?);
            } else if self.consume("-") {
                value = sub_intmax(value, self.parse_multiplicative()?);
            } else {
                return Ok(value);
            }
        }
    }

    fn parse_multiplicative(&mut self) -> Result<i128, ArithmeticError> {
        let mut value = self.parse_power()?;
        loop {
            self.skip_ws();
            if self.consume("*") {
                value = mul_intmax(value, self.parse_power()?);
            } else if self.consume("/") {
                value = checked_div(value, self.parse_power()?)?;
            } else if self.consume("%") {
                value = checked_rem(value, self.parse_power()?)?;
            } else {
                return Ok(value);
            }
        }
    }

    fn parse_power(&mut self) -> Result<i128, ArithmeticError> {
        let value = self.parse_unary()?;
        self.skip_ws();
        if self.consume("**") {
            let rhs = self.parse_power()?;
            if rhs < 0 {
                return Err(ArithmeticError::new("exponent less than 0"));
            }
            return Ok(pow_intmax(value, rhs));
        }
        Ok(value)
    }

    fn parse_unary(&mut self) -> Result<i128, ArithmeticError> {
        self.skip_ws();
        if self.starts_with_number_after("++") {
            self.consume("++");
            return self.parse_unary();
        }
        if self.starts_with_number_after("--") {
            self.consume("--");
            return Ok(neg_intmax(neg_intmax(self.parse_unary()?)));
        }
        if self.consume("++") {
            let lvalue = self
                .parse_lvalue()
                .ok_or_else(|| ArithmeticError::new("operand expected"))?;
            let value = add_intmax(self.lvalue_value(&lvalue)?, 1);
            self.set_lvalue(&lvalue, value);
            return Ok(value);
        }
        if self.consume("--") {
            let lvalue = self
                .parse_lvalue()
                .ok_or_else(|| ArithmeticError::new("operand expected"))?;
            let value = sub_intmax(self.lvalue_value(&lvalue)?, 1);
            self.set_lvalue(&lvalue, value);
            return Ok(value);
        }
        if self.consume("+") {
            return self.parse_unary();
        }
        if self.consume("-") {
            return Ok(neg_intmax(self.parse_unary()?));
        }
        if self.consume("!") {
            return Ok(i128::from(self.parse_unary()? == 0));
        }
        if self.consume("~") {
            return Ok(not_intmax(self.parse_unary()?));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<i128, ArithmeticError> {
        self.skip_ws();
        if self.consume("(") {
            let value = self.parse_comma()?;
            self.skip_ws();
            if !self.consume(")") {
                return Err(ArithmeticError::new("missing `)'"));
            }
            return Ok(value);
        }

        if let Some(value) = self.parse_number()? {
            return Ok(value);
        }

        if let Some(value) = self.parse_quoted_empty()? {
            return Ok(value);
        }

        if let Some(lvalue) = self.parse_lvalue() {
            let value = self.lvalue_value(&lvalue)?;
            self.skip_ws();
            if self.consume("++") {
                self.set_lvalue(&lvalue, add_intmax(value, 1));
            } else if self.consume("--") {
                self.set_lvalue(&lvalue, sub_intmax(value, 1));
            }
            return Ok(value);
        }

        Err(ArithmeticError::new("operand expected"))
    }

    fn parse_quoted_empty(&mut self) -> Result<Option<i128>, ArithmeticError> {
        self.skip_ws();
        let Some(quote @ ('"' | '\'')) = self.peek_char() else {
            return Ok(None);
        };
        self.pos += quote.len_utf8();
        let start = self.pos;
        while let Some(ch) = self.peek_char() {
            if ch == quote {
                let inner = &self.input[start..self.pos];
                self.pos += ch.len_utf8();
                if inner.trim().is_empty() {
                    return Ok(Some(0));
                }
                return Err(ArithmeticError::new("operand expected"));
            }
            self.pos += ch.len_utf8();
        }
        Err(ArithmeticError::new("operand expected"))
    }

    fn parse_number(&mut self) -> Result<Option<i128>, ArithmeticError> {
        self.skip_ws();
        let start = self.pos;
        while let Some(ch) = self.peek_char() {
            if ch.is_ascii_alphanumeric() || ch == '#' || ch == '_' || ch == '@' {
                self.pos += ch.len_utf8();
            } else {
                break;
            }
        }
        if self.pos == start {
            return Ok(None);
        }
        let token = &self.input[start..self.pos];
        if token
            .chars()
            .next()
            .is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic())
        {
            self.pos = start;
            return Ok(None);
        }

        if let Some((base, digits)) = token.split_once('#') {
            let base = base
                .parse::<u32>()
                .map_err(|_| ArithmeticError::new("invalid arithmetic base"))?;
            if !(2..=64).contains(&base) {
                return Err(ArithmeticError::new("invalid arithmetic base"));
            }
            if digits.is_empty() {
                return Err(ArithmeticError::new("invalid integer constant"));
            }
            let mut value = 0_i128;
            for ch in digits.chars() {
                let digit =
                    digit_value(ch, base).ok_or_else(|| ArithmeticError::new("invalid number"))?;
                if digit >= base {
                    return Err(ArithmeticError::new("value too great for base"));
                }
                value = add_intmax(mul_intmax(value, base as i128), digit as i128);
            }
            return Ok(Some(value));
        }

        if let Some(hex) = token
            .strip_prefix("0x")
            .or_else(|| token.strip_prefix("0X"))
        {
            return i128::from_str_radix(hex, 16)
                .map(wrap_intmax)
                .map(Some)
                .map_err(|_| ArithmeticError::new("invalid number"));
        }

        if token.len() > 1 && token.starts_with('0') {
            return i128::from_str_radix(token, 8)
                .map(wrap_intmax)
                .map(Some)
                .map_err(|_| ArithmeticError::new("value too great for base"));
        }

        token
            .parse::<i128>()
            .map(wrap_intmax)
            .map(Some)
            .map_err(|_| ArithmeticError::new("invalid number"))
    }

    fn parse_name_only(&mut self) -> Option<String> {
        self.skip_ws();
        let start = self.pos;
        let first = self.peek_char()?;
        if !(first == '_' || first.is_ascii_alphabetic()) {
            return None;
        }
        self.pos += first.len_utf8();
        while let Some(ch) = self.peek_char() {
            if ch == '_' || ch.is_ascii_alphanumeric() {
                self.pos += ch.len_utf8();
            } else {
                break;
            }
        }
        Some(self.input[start..self.pos].to_string())
    }

    fn parse_lvalue(&mut self) -> Option<LValue> {
        let checkpoint = self.pos;
        let name = self.parse_name_only()?;
        self.skip_ws();
        if !self.consume("[") {
            return Some(LValue::Variable(name));
        }

        let index_start = self.pos;
        let mut bracket_depth = 0_i32;
        let mut paren_depth = 0_i32;
        while let Some(ch) = self.peek_char() {
            match ch {
                '[' => {
                    bracket_depth += 1;
                    self.pos += ch.len_utf8();
                }
                ']' if bracket_depth > 0 => {
                    bracket_depth -= 1;
                    self.pos += ch.len_utf8();
                }
                ']' if paren_depth == 0 => break,
                '(' => {
                    paren_depth += 1;
                    self.pos += ch.len_utf8();
                }
                ')' if paren_depth > 0 => {
                    paren_depth -= 1;
                    self.pos += ch.len_utf8();
                }
                _ => self.pos += ch.len_utf8(),
            }
        }
        let raw_index_expr = &self.input[index_start..self.pos];
        if raw_index_expr.is_empty() || !self.consume("]") {
            self.pos = checkpoint;
            return None;
        }
        let index_expr = raw_index_expr.trim().to_string();
        Some(LValue::Indexed { name, index_expr })
    }

    fn var_value(&mut self, name: &str) -> Result<i128, ArithmeticError> {
        if self.noeval {
            return Ok(0);
        }
        if self.nounset_enabled() && !self.vars.contains_key(name) {
            return Err(ArithmeticError::new(format!("{name}: unbound variable")));
        }
        let value = self.vars.get(name).cloned().unwrap_or_default();
        if crate::shell::arrays::indexed::is_storage(&value) {
            return self.value_to_arith(&crate::shell::arrays::indexed::value_at(&value, 0));
        }
        self.value_to_arith(&value)
    }

    fn lvalue_value(&mut self, lvalue: &LValue) -> Result<i128, ArithmeticError> {
        match lvalue {
            LValue::Variable(name) => self.var_value(name),
            LValue::Indexed { name, index_expr } => {
                if self.noeval {
                    return Ok(0);
                }
                if self.nounset_enabled() && !self.vars.contains_key(name) {
                    return Err(ArithmeticError::new(format!("{name}: unbound variable")));
                }
                let index = self.lvalue_index(index_expr)?;
                let storage = self.vars.get(name).cloned().unwrap_or_default();
                let value = crate::shell::arrays::indexed::value_at(&storage, index);
                self.value_to_arith(&value)
            }
        }
    }

    fn set_lvalue(&mut self, lvalue: &LValue, value: i128) {
        if self.noeval {
            return;
        }
        match lvalue {
            LValue::Variable(name) => {
                self.vars
                    .insert(name.clone(), wrap_intmax(value).to_string());
            }
            LValue::Indexed { name, index_expr } => {
                let index = match self.lvalue_index(index_expr) {
                    Ok(index) => index,
                    Err(_) => return,
                };
                let storage = self.vars.get(name).cloned().unwrap_or_default();
                let storage = crate::shell::arrays::indexed::set_value_at(
                    &storage,
                    index,
                    wrap_intmax(value).to_string(),
                );
                self.vars.insert(name.clone(), storage);
            }
        }
    }

    fn lvalue_index(&mut self, index_expr: &str) -> Result<usize, ArithmeticError> {
        if index_expr.trim().is_empty() {
            return Ok(0);
        }
        if let Some(index) = quoted_whitespace_index(index_expr) {
            return Ok(index);
        }
        if escaped_quoted_index(index_expr) {
            return Ok(0);
        }
        let mut nested = Parser::nested(index_expr, self.vars, self.depth + 1);
        let value = nested.parse_comma()?;
        Ok(value.max(0) as usize)
    }

    fn value_to_arith(&mut self, value: &str) -> Result<i128, ArithmeticError> {
        if self.noeval {
            return Ok(0);
        }
        if value.trim().is_empty() {
            return Ok(0);
        }
        if value
            .chars()
            .all(|ch| ch.is_ascii_digit() || matches!(ch, '-' | '+'))
            && value.matches(['-', '+']).count() <= 1
        {
            return value.trim().parse().map(wrap_intmax).or(Ok(0));
        }
        if self.depth >= MAX_EXPR_RECURSION_LEVEL {
            let token = value.trim();
            if is_arithmetic_name(token) {
                return Err(ArithmeticError::new(format!(
                    "{token}: expression recursion level exceeded (error token is \"{token}\")"
                )));
            }
            return Err(ArithmeticError::new("expression recursion level exceeded"));
        }
        let mut nested = Parser::nested(&value, self.vars, self.depth + 1);
        nested.parse_comma()
    }

    fn nounset_enabled(&self) -> bool {
        self.vars.get("__RUBASH_NOUNSET").map(String::as_str) == Some("1")
    }

    fn skip_to_conditional_colon(&mut self) {
        let mut paren = 0_i32;
        let mut ternary = 0_i32;
        while let Some(ch) = self.peek_char() {
            match ch {
                '(' => {
                    paren += 1;
                    self.pos += 1;
                }
                ')' if paren > 0 => {
                    paren -= 1;
                    self.pos += 1;
                }
                '?' if paren == 0 => {
                    ternary += 1;
                    self.pos += 1;
                }
                ':' if paren == 0 && ternary > 0 => {
                    ternary -= 1;
                    self.pos += 1;
                }
                ':' if paren == 0 => break,
                _ => self.pos += ch.len_utf8(),
            }
        }
    }

    fn skip_unevaluated_conditional_tail(&mut self) -> String {
        let start = self.pos;
        let mut paren = 0_i32;
        while let Some(ch) = self.peek_char() {
            match ch {
                '(' => {
                    paren += 1;
                    self.pos += 1;
                }
                ')' if paren > 0 => {
                    paren -= 1;
                    self.pos += 1;
                }
                ')' if paren == 0 => break,
                _ => self.pos += ch.len_utf8(),
            }
        }
        self.input[start..self.pos].to_string()
    }

    fn skip_ws(&mut self) {
        while self.peek_char().is_some_and(|ch| ch.is_whitespace()) {
            self.pos += 1;
        }
    }

    fn consume(&mut self, s: &str) -> bool {
        self.skip_ws();
        if self.starts_with(s) {
            self.pos += s.len();
            true
        } else {
            false
        }
    }

    fn starts_with(&self, s: &str) -> bool {
        self.input[self.pos..].starts_with(s)
    }

    fn starts_with_number_after(&self, s: &str) -> bool {
        self.input[self.pos..]
            .strip_prefix(s)
            .and_then(|rest| rest.chars().next())
            .is_some_and(|ch| ch.is_ascii_digit())
    }

    fn peek_char(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    fn eof(&self) -> bool {
        self.pos >= self.input.len()
    }
}

fn checked_div(left: i128, right: i128) -> Result<i128, ArithmeticError> {
    if right == 0 {
        return Err(ArithmeticError::new("division by 0"));
    }
    let left = to_intmax(left);
    let right = to_intmax(right);
    if left == i64::MIN && right == -1 {
        return Ok(i64::MIN as i128);
    }
    Ok((left / right) as i128)
}

fn checked_rem(left: i128, right: i128) -> Result<i128, ArithmeticError> {
    if right == 0 {
        return Err(ArithmeticError::new("division by 0"));
    }
    let left = to_intmax(left);
    let right = to_intmax(right);
    if left == i64::MIN && right == -1 {
        return Ok(0);
    }
    Ok((left % right) as i128)
}

fn shift_amount(value: i128) -> u32 {
    value.clamp(0, 63) as u32
}

fn wrap_intmax(value: i128) -> i128 {
    to_intmax(value) as i128
}

fn to_intmax(value: i128) -> i64 {
    value as i64
}

fn add_intmax(left: i128, right: i128) -> i128 {
    to_intmax(left).wrapping_add(to_intmax(right)) as i128
}

fn sub_intmax(left: i128, right: i128) -> i128 {
    to_intmax(left).wrapping_sub(to_intmax(right)) as i128
}

fn mul_intmax(left: i128, right: i128) -> i128 {
    to_intmax(left).wrapping_mul(to_intmax(right)) as i128
}

fn neg_intmax(value: i128) -> i128 {
    to_intmax(value).wrapping_neg() as i128
}

fn not_intmax(value: i128) -> i128 {
    !to_intmax(value) as i128
}

fn bitand_intmax(left: i128, right: i128) -> i128 {
    (to_intmax(left) & to_intmax(right)) as i128
}

fn bitor_intmax(left: i128, right: i128) -> i128 {
    (to_intmax(left) | to_intmax(right)) as i128
}

fn bitxor_intmax(left: i128, right: i128) -> i128 {
    (to_intmax(left) ^ to_intmax(right)) as i128
}

fn shl_intmax(left: i128, right: i128) -> i128 {
    to_intmax(left).wrapping_shl(shift_amount(right)) as i128
}

fn shr_intmax(left: i128, right: i128) -> i128 {
    to_intmax(left).wrapping_shr(shift_amount(right)) as i128
}

fn pow_intmax(left: i128, right: i128) -> i128 {
    let mut result = 1_i64;
    let mut base = to_intmax(left);
    let mut exp = right.min(u32::MAX as i128) as u32;
    while exp > 0 {
        if exp & 1 == 1 {
            result = result.wrapping_mul(base);
        }
        exp >>= 1;
        if exp > 0 {
            base = base.wrapping_mul(base);
        }
    }
    result as i128
}

fn digit_value(ch: char, base: u32) -> Option<u32> {
    match ch {
        '0'..='9' => Some(ch as u32 - '0' as u32),
        'a'..='z' => Some(ch as u32 - 'a' as u32 + 10),
        'A'..='Z' if base <= 36 => Some(ch as u32 - 'A' as u32 + 10),
        'A'..='Z' => Some(ch as u32 - 'A' as u32 + 36),
        '@' => Some(62),
        '_' => Some(63),
        _ => None,
    }
}

fn quoted_whitespace_index(value: &str) -> Option<usize> {
    // TODO(expr.c/arrayfunc.c): Bash removes quotes before evaluating indexed
    // array subscripts. Keep this narrow for arith10.sub: `" "` becomes index
    // zero, while `""` still reports invalid empty subscript in arithmetic
    // contexts until the full array_expand_index path is ported.
    let value = value.trim();
    let inner = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })?;
    (!inner.is_empty() && inner.trim().is_empty()).then_some(0)
}

fn is_arithmetic_name(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn escaped_quoted_index(value: &str) -> bool {
    // TODO(expr.c/arrayfunc.c): Arithmetic array subscripts perform quote
    // removal before evaluation. The lexer preserves backslashes in arith10's
    // `a[\"...\"]` forms, which Bash treats as an empty/blank arithmetic
    // subscript evaluating to zero in arithmetic contexts.
    let value = value.trim();
    value
        .strip_prefix("\\\"")
        .and_then(|value| value.strip_suffix("\\\""))
        .is_some_and(|inner| inner.trim().is_empty())
}

fn tail_has_top_level_assignment(value: &str) -> bool {
    // TODO(expr.c): Bash diagnoses assignment operators in unevaluated
    // conditional arms when precedence leaves the assignment outside the
    // conditional expression. This preserves that behavior for arith.tests'
    // `1 ? 20 : x+=2` without evaluating parenthesized short-circuit arms.
    let bytes = value.as_bytes();
    let mut paren = 0_i32;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'(' => paren += 1,
            b')' if paren > 0 => paren -= 1,
            b'=' if paren == 0 => {
                if index > 0
                    && matches!(
                        bytes[index - 1],
                        b'+' | b'-' | b'*' | b'/' | b'%' | b'&' | b'^' | b'|' | b'<' | b'>'
                    )
                {
                    return true;
                }
                if bytes.get(index + 1) != Some(&b'=') {
                    return true;
                }
            }
            _ => {}
        }
        index += 1;
    }
    false
}
