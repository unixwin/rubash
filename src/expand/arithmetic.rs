//! arithmetic module.
//!
//! GNU Bash source ownership:
// - expr.c

use std::collections::HashMap;

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
    let value = parser.parse_assignment()?;
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
}

#[derive(Clone, Debug)]
enum LValue {
    Variable(String),
    Indexed { name: String, index: usize },
}

impl<'a> Parser<'a> {
    fn new(input: &'a str, vars: &'a mut HashMap<String, String>) -> Self {
        Self {
            input,
            pos: 0,
            vars,
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
                    let current = self.lvalue_value(&lvalue)?;
                    let value = match op {
                        "=" => rhs,
                        "+=" => current + rhs,
                        "-=" => current - rhs,
                        "*=" => current * rhs,
                        "/=" => checked_div(current, rhs)?,
                        "%=" => checked_rem(current, rhs)?,
                        "<<=" => current << shift_amount(rhs),
                        ">>=" => current >> shift_amount(rhs),
                        "&=" => current & rhs,
                        "^=" => current ^ rhs,
                        "|=" => current | rhs,
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
            self.skip_unevaluated_conditional_tail();
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
            let rhs = self.parse_logical_and()?;
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
            let rhs = self.parse_bit_or()?;
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
            value |= self.parse_bit_xor()?;
        }
    }

    fn parse_bit_xor(&mut self) -> Result<i128, ArithmeticError> {
        let mut value = self.parse_bit_and()?;
        loop {
            self.skip_ws();
            if !self.consume("^") {
                return Ok(value);
            }
            value ^= self.parse_bit_and()?;
        }
    }

    fn parse_bit_and(&mut self) -> Result<i128, ArithmeticError> {
        let mut value = self.parse_equality()?;
        loop {
            self.skip_ws();
            if self.starts_with("&&") || !self.consume("&") {
                return Ok(value);
            }
            value &= self.parse_equality()?;
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
                value <<= shift_amount(self.parse_additive()?);
            } else if self.consume(">>") {
                value >>= shift_amount(self.parse_additive()?);
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
                value += self.parse_multiplicative()?;
            } else if self.consume("-") {
                value -= self.parse_multiplicative()?;
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
                value *= self.parse_power()?;
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
            return Ok(value.pow(rhs.min(u32::MAX as i128) as u32));
        }
        Ok(value)
    }

    fn parse_unary(&mut self) -> Result<i128, ArithmeticError> {
        self.skip_ws();
        if self.consume("++") {
            let lvalue = self
                .parse_lvalue()
                .ok_or_else(|| ArithmeticError::new("operand expected"))?;
            let value = self.lvalue_value(&lvalue)? + 1;
            self.set_lvalue(&lvalue, value);
            return Ok(value);
        }
        if self.consume("--") {
            let lvalue = self
                .parse_lvalue()
                .ok_or_else(|| ArithmeticError::new("operand expected"))?;
            let value = self.lvalue_value(&lvalue)? - 1;
            self.set_lvalue(&lvalue, value);
            return Ok(value);
        }
        if self.consume("+") {
            return self.parse_unary();
        }
        if self.consume("-") {
            return Ok(-self.parse_unary()?);
        }
        if self.consume("!") {
            return Ok(i128::from(self.parse_unary()? == 0));
        }
        if self.consume("~") {
            return Ok(!self.parse_unary()?);
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<i128, ArithmeticError> {
        self.skip_ws();
        if self.consume("(") {
            let value = self.parse_assignment()?;
            self.skip_ws();
            if !self.consume(")") {
                return Err(ArithmeticError::new("missing `)'"));
            }
            return Ok(value);
        }

        if let Some(value) = self.parse_number()? {
            return Ok(value);
        }

        if let Some(lvalue) = self.parse_lvalue() {
            let value = self.lvalue_value(&lvalue)?;
            self.skip_ws();
            if self.consume("++") {
                self.set_lvalue(&lvalue, value + 1);
            } else if self.consume("--") {
                self.set_lvalue(&lvalue, value - 1);
            }
            return Ok(value);
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
                value = value * base as i128 + digit as i128;
            }
            return Ok(Some(value));
        }

        if let Some(hex) = token
            .strip_prefix("0x")
            .or_else(|| token.strip_prefix("0X"))
        {
            return i128::from_str_radix(hex, 16)
                .map(Some)
                .map_err(|_| ArithmeticError::new("invalid number"));
        }

        if token.len() > 1 && token.starts_with('0') {
            return i128::from_str_radix(token, 8)
                .map(Some)
                .map_err(|_| ArithmeticError::new("value too great for base"));
        }

        token
            .parse::<i128>()
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
        let index = match self.parse_assignment() {
            Ok(value) if value >= 0 => value as usize,
            _ => {
                self.pos = checkpoint;
                return None;
            }
        };
        self.skip_ws();
        if !self.consume("]") {
            self.pos = checkpoint;
            return None;
        }
        Some(LValue::Indexed { name, index })
    }

    fn var_value(&mut self, name: &str) -> Result<i128, ArithmeticError> {
        let value = self.vars.get(name).cloned().unwrap_or_default();
        self.value_to_arith(&value)
    }

    fn lvalue_value(&mut self, lvalue: &LValue) -> Result<i128, ArithmeticError> {
        match lvalue {
            LValue::Variable(name) => self.var_value(name),
            LValue::Indexed { name, index } => {
                let storage = self.vars.get(name).cloned().unwrap_or_default();
                let value = crate::shell::arrays::indexed::value_at(&storage, *index);
                self.value_to_arith(&value)
            }
        }
    }

    fn set_lvalue(&mut self, lvalue: &LValue, value: i128) {
        match lvalue {
            LValue::Variable(name) => {
                self.vars.insert(name.clone(), value.to_string());
            }
            LValue::Indexed { name, index } => {
                let storage = self.vars.get(name).cloned().unwrap_or_default();
                let storage = crate::shell::arrays::indexed::set_value_at(
                    &storage,
                    *index,
                    value.to_string(),
                );
                self.vars.insert(name.clone(), storage);
            }
        }
    }

    fn value_to_arith(&mut self, value: &str) -> Result<i128, ArithmeticError> {
        if value.trim().is_empty() {
            return Ok(0);
        }
        if value
            .chars()
            .all(|ch| ch.is_ascii_digit() || matches!(ch, '-' | '+'))
            && value.matches(['-', '+']).count() <= 1
        {
            return value.trim().parse().or(Ok(0));
        }
        let mut nested = Parser::new(&value, self.vars);
        nested.parse_assignment()
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

    fn skip_unevaluated_conditional_tail(&mut self) {
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
    Ok(left / right)
}

fn checked_rem(left: i128, right: i128) -> Result<i128, ArithmeticError> {
    if right == 0 {
        return Err(ArithmeticError::new("division by 0"));
    }
    Ok(left % right)
}

fn shift_amount(value: i128) -> u32 {
    value.clamp(0, 127) as u32
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
