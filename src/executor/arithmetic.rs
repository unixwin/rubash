//! Arithmetic expression parsing and evaluation.
//!
//! Provides parsing and evaluation of shell arithmetic expressions including
//! variables, arrays, assignments, and ternary conditionals.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};

use super::{
    array_value_at, assoc_entries, assoc_value_at, format_assoc_storage,
    format_indexed_array_storage, indexed_array_entries, is_marked_var, is_noassign_bash_array,
    is_shell_name, is_shell_name_char, is_shell_name_start, mark_env_name, next_random_from_state,
    resolve_indexed_array_subscript, set_process_env, strip_matching_quotes, Executor,
    ARRAY_VARS, ASSOC_VARS, NAMEREF_VARS,
};

impl Executor {
    pub(crate) fn eval_arithmetic_command_value(&mut self, expression: &str) -> Option<i128> {
        let expression = self.expand_arithmetic_special_parameters(expression);
        eval_mutable_arith_value_with_random(
            &expression,
            &mut self.env_vars,
            Some(&self.random_state),
        )
    }

    pub(super) fn expand_arithmetic_special_parameters(&self, expression: &str) -> String {
        let expression = expression.replace("$#", &self.positional_params.len().to_string());
        self.expand_embedded_parameters(&expression)
    }
}

pub(super) fn eval_arith_value(value: &str) -> i128 {
    value
        .split('+')
        .map(|part| part.trim().parse::<i128>().unwrap_or(0))
        .sum()
}

pub(super) fn eval_conditional_arith_value(value: &str, env_vars: &HashMap<String, String>) -> Option<i128> {
    let mut env_vars = env_vars.clone();
    eval_mutable_arith_value(value, &mut env_vars)
}

pub(super) fn arithmetic_division_by_zero_token(expression: &str) -> Option<&'static str> {
    let bytes = expression.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if !matches!(bytes[index], b'/' | b'%') {
            index += 1;
            continue;
        }
        index += 1;
        while bytes
            .get(index)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            index += 1;
        }
        if matches!(bytes.get(index), Some(b'+' | b'-')) {
            index += 1;
        }
        let start = index;
        while bytes.get(index).is_some_and(|byte| byte.is_ascii_digit()) {
            index += 1;
        }
        if start != index
            && expression[start..index]
                .parse::<i128>()
                .is_ok_and(|value| value == 0)
        {
            return Some("0 ");
        }
    }
    None
}

fn eval_mutable_arith_value(value: &str, env_vars: &mut HashMap<String, String>) -> Option<i128> {
    eval_mutable_arith_value_with_random(value, env_vars, None)
}

pub(super) fn eval_mutable_arith_value_with_random(
    value: &str,
    env_vars: &mut HashMap<String, String>,
    random_state: Option<&Cell<u32>>,
) -> Option<i128> {
    let mut parser = ConditionalArithParser {
        input: value.as_bytes(),
        pos: 0,
        env_vars,
        resolving: Vec::new(),
        random_state,
    };
    let value = parser.parse_comma()?;
    parser.skip_ws();
    (parser.pos == parser.input.len()).then_some(value)
}

fn bash_arith(value: i128) -> i128 {
    value as i64 as i128
}

fn checked_arithmetic_pow(base: i128, exponent: i128) -> Option<i128> {
    let exponent = u32::try_from(exponent).ok()?;
    let mut value = 1i128;
    for _ in 0..exponent {
        value = bash_arith(value * base);
    }
    Some(value)
}

fn parse_arithmetic_digits(digits: &[u8], base: u32) -> Option<i128> {
    let mut value = 0i128;
    for digit in std::str::from_utf8(digits).ok()?.chars() {
        let digit = arithmetic_digit_value(digit, base)?;
        if digit >= base {
            return None;
        }
        value = bash_arith(value * i128::from(base) + i128::from(digit));
    }
    Some(value)
}

fn arithmetic_digit_value(ch: char, base: u32) -> Option<u32> {
    match ch {
        '0'..='9' => Some(ch as u32 - '0' as u32),
        'a'..='z' => Some(10 + ch as u32 - 'a' as u32),
        'A'..='Z' if base <= 36 => Some(10 + ch as u32 - 'A' as u32),
        'A'..='Z' => Some(36 + ch as u32 - 'A' as u32),
        '@' => Some(62),
        '_' => Some(63),
        _ => None,
    }
}

fn skip_arith_ws(input: &[u8], pos: &mut usize) {
    while input.get(*pos).is_some_and(|ch| ch.is_ascii_whitespace()) {
        *pos += 1;
    }
}

fn assignment_operator_at(input: &[u8], pos: usize) -> Option<&'static str> {
    for op in [
        "<<=", ">>=", "**=", "+=", "-=", "*=", "/=", "%=", "&=", "^=", "|=", "=",
    ] {
        if op == "="
            && (input.get(pos + 1) == Some(&b'=')
                || (pos > 0 && matches!(input.get(pos - 1), Some(b'!') | Some(b'<') | Some(b'>'))))
        {
            continue;
        }
        if input
            .get(pos..)
            .is_some_and(|rest| rest.starts_with(op.as_bytes()))
        {
            return Some(op);
        }
    }
    None
}

struct ConditionalArithParser<'a> {
    input: &'a [u8],
    pos: usize,
    env_vars: &'a mut HashMap<String, String>,
    resolving: Vec<String>,
    random_state: Option<&'a Cell<u32>>,
}

#[derive(Clone)]
enum ArithLValue {
    Scalar(String),
    Indexed { name: String, index: i128 },
    Assoc { name: String, key: String },
}

impl ConditionalArithParser<'_> {
    fn parse_comma(&mut self) -> Option<i128> {
        let mut value = self.parse_assignment()?;
        loop {
            self.skip_ws();
            if !self.consume(",") {
                return Some(value);
            }
            value = self.parse_assignment()?;
        }
    }

    fn parse_assignment(&mut self) -> Option<i128> {
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

    fn assignment_lvalue_is_next(&self) -> bool {
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

    fn parse_conditional(&mut self) -> Option<i128> {
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

    fn parse_logical_or(&mut self) -> Option<i128> {
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

    fn parse_logical_and(&mut self) -> Option<i128> {
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

    fn parse_bitwise_or(&mut self) -> Option<i128> {
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

    fn parse_bitwise_xor(&mut self) -> Option<i128> {
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

    fn parse_bitwise_and(&mut self) -> Option<i128> {
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

    fn parse_comparison(&mut self) -> Option<i128> {
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

    fn parse_shift(&mut self) -> Option<i128> {
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

    fn parse_expr(&mut self) -> Option<i128> {
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

    fn parse_term(&mut self) -> Option<i128> {
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

    fn parse_power(&mut self) -> Option<i128> {
        let value = self.parse_factor()?;
        self.skip_ws();
        if self.consume("**") {
            let rhs = self.parse_power()?;
            checked_arithmetic_pow(value, rhs)
        } else {
            Some(value)
        }
    }

    fn parse_factor(&mut self) -> Option<i128> {
        self.skip_ws();
        if self.consume("++") {
            let lvalue = self.parse_lvalue()?;
            return self.update_lvalue(&lvalue, 1, true);
        }
        if self.consume("--") {
            let lvalue = self.parse_lvalue()?;
            return self.update_lvalue(&lvalue, -1, true);
        }
        match self.peek()? {
            b'+' => {
                self.pos += 1;
                self.parse_factor()
            }
            b'-' => {
                self.pos += 1;
                self.parse_factor().map(|value| bash_arith(-value))
            }
            b'!' => {
                self.pos += 1;
                self.parse_factor().map(|value| i128::from(value == 0))
            }
            b'~' => {
                self.pos += 1;
                self.parse_factor().map(|value| bash_arith(!value))
            }
            b'(' => {
                self.pos += 1;
                let value = self.parse_comma()?;
                self.skip_ws();
                (self.peek()? == b')').then(|| self.pos += 1)?;
                Some(value)
            }
            b'$' => self.parse_dollar_variable(),
            ch if ch.is_ascii_digit() => self.parse_number(),
            ch if is_shell_name_start(ch as char) => self.parse_variable(),
            _ => None,
        }
    }

    fn parse_number(&mut self) -> Option<i128> {
        let start = self.pos;
        while self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
            self.pos += 1;
        }
        if self.peek() == Some(b'#') {
            let base_text = std::str::from_utf8(&self.input[start..self.pos]).ok()?;
            let base = base_text.parse::<u32>().ok()?;
            if !(2..=64).contains(&base) {
                return None;
            }
            self.pos += 1;
            let digit_start = self.pos;
            while self.peek().is_some_and(|ch| {
                arithmetic_digit_value(ch as char, base).is_some_and(|value| value < base)
            }) {
                self.pos += 1;
            }
            if self.pos == digit_start {
                return None;
            }
            return parse_arithmetic_digits(&self.input[digit_start..self.pos], base);
        }

        if self.input[start..].starts_with(b"0x") || self.input[start..].starts_with(b"0X") {
            self.pos = start + 2;
            let digit_start = self.pos;
            while self.peek().is_some_and(|ch| ch.is_ascii_hexdigit()) {
                self.pos += 1;
            }
            if self.pos == digit_start {
                return None;
            }
            return parse_arithmetic_digits(&self.input[digit_start..self.pos], 16);
        }

        let text = std::str::from_utf8(&self.input[start..self.pos]).ok()?;
        let base = if text.len() > 1 && text.starts_with('0') {
            8
        } else {
            10
        };
        parse_arithmetic_digits(text.as_bytes(), base)
    }

    fn parse_dollar_variable(&mut self) -> Option<i128> {
        self.pos += 1;
        if self.consume("{") {
            let start = self.pos;
            let first = self.peek()? as char;
            if !is_shell_name_start(first) {
                return None;
            }
            self.pos += 1;
            while self.peek().is_some_and(|ch| is_shell_name_char(ch as char)) {
                self.pos += 1;
            }
            let name = std::str::from_utf8(&self.input[start..self.pos])
                .ok()?
                .to_string();
            self.skip_ws();
            if !self.consume("}") {
                return None;
            }
            return self.variable_value(&name);
        }

        let start = self.pos;
        let first = self.peek()? as char;
        if !is_shell_name_start(first) {
            return None;
        }
        self.pos += 1;
        while self.peek().is_some_and(|ch| is_shell_name_char(ch as char)) {
            self.pos += 1;
        }
        let name = std::str::from_utf8(&self.input[start..self.pos]).ok()?;
        self.variable_value(name)
    }

    fn parse_variable(&mut self) -> Option<i128> {
        let lvalue = self.parse_lvalue()?;
        if self.consume("++") {
            return self.update_lvalue(&lvalue, 1, false);
        }
        if self.consume("--") {
            return self.update_lvalue(&lvalue, -1, false);
        }
        self.lvalue_value(&lvalue)
    }

    fn parse_lvalue(&mut self) -> Option<ArithLValue> {
        self.skip_ws();
        let start = self.pos;
        let first = self.peek()? as char;
        if !is_shell_name_start(first) {
            return None;
        }
        self.pos += 1;
        while self.peek().is_some_and(|ch| is_shell_name_char(ch as char)) {
            self.pos += 1;
        }
        let name = std::str::from_utf8(&self.input[start..self.pos])
            .ok()?
            .to_string();

        self.skip_ws();
        if !self.consume("[") {
            let name = self.resolved_lvalue_name(&name);
            return Some(ArithLValue::Scalar(name));
        }

        let resolved_name = self.resolved_lvalue_name(&name);
        if is_marked_var(self.env_vars, ASSOC_VARS, &resolved_name) {
            let key = self.parse_assoc_subscript()?;
            return Some(ArithLValue::Assoc {
                name: resolved_name,
                key,
            });
        }

        let index = self.parse_comma()?;
        self.skip_ws();
        if !self.consume("]") {
            return None;
        }
        Some(ArithLValue::Indexed {
            name: resolved_name,
            index,
        })
    }

    fn resolved_lvalue_name(&self, name: &str) -> String {
        let mut current = name;
        let mut seen = HashSet::new();
        for _ in 0..16 {
            if !seen.insert(current.to_string()) {
                return name.to_string();
            }
            if !is_marked_var(self.env_vars, NAMEREF_VARS, current) {
                return current.to_string();
            }
            let Some(target) = self.env_vars.get(current) else {
                return current.to_string();
            };
            if !is_shell_name(target) {
                return current.to_string();
            }
            current = target;
        }
        name.to_string()
    }

    fn parse_assoc_subscript(&mut self) -> Option<String> {
        let start = self.pos;
        let mut depth = 0usize;
        while self.pos < self.input.len() {
            match self.input[self.pos] {
                b'[' => {
                    depth += 1;
                    self.pos += 1;
                }
                b']' if depth == 0 => {
                    let key = std::str::from_utf8(&self.input[start..self.pos])
                        .ok()?
                        .trim()
                        .to_string();
                    self.pos += 1;
                    return Some(self.expand_assoc_subscript_key(&key));
                }
                b']' => {
                    depth -= 1;
                    self.pos += 1;
                }
                _ => self.pos += 1,
            }
        }
        None
    }

    fn expand_assoc_subscript_key(&self, key: &str) -> String {
        let mut output = String::new();
        let mut chars = key.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch != '$' {
                output.push(ch);
                continue;
            }

            match chars.peek().copied() {
                Some('{') => {
                    chars.next();
                    let mut name = String::new();
                    for name_ch in chars.by_ref() {
                        if name_ch == '}' {
                            break;
                        }
                        name.push(name_ch);
                    }
                    output.push_str(self.env_vars.get(&name).map(String::as_str).unwrap_or(""));
                }
                Some(first) if is_shell_name_start(first) => {
                    chars.next();
                    let mut name = String::from(first);
                    while chars.peek().copied().is_some_and(is_shell_name_char) {
                        name.push(chars.next().unwrap());
                    }
                    output.push_str(self.env_vars.get(&name).map(String::as_str).unwrap_or(""));
                }
                _ => output.push(ch),
            }
        }

        strip_matching_quotes(output.trim()).to_string()
    }

    fn consume_assignment_operator(&mut self) -> Option<&'static str> {
        let op = assignment_operator_at(self.input, self.pos)?;
        self.pos += op.len();
        Some(op)
    }

    fn lvalue_value(&mut self, lvalue: &ArithLValue) -> Option<i128> {
        match lvalue {
            ArithLValue::Scalar(name) => self.variable_value(name),
            ArithLValue::Indexed { name, index } => {
                let value = self.env_vars.get(name).and_then(|value| {
                    resolve_indexed_array_subscript(value, *index)
                        .and_then(|index| array_value_at(value, index))
                });
                let value = value.unwrap_or_default();
                self.evaluate_variable_text(&format!("{name}[{index}]"), &value)
            }
            ArithLValue::Assoc { name, key } => {
                let value = self
                    .env_vars
                    .get(name)
                    .and_then(|value| assoc_value_at(value, key))
                    .unwrap_or_default();
                self.evaluate_variable_text(&format!("{name}[{key}]"), &value)
            }
        }
    }

    fn variable_value(&mut self, name: &str) -> Option<i128> {
        if self.resolving.iter().any(|resolving| resolving == name) {
            return None;
        }
        if name == "RANDOM" {
            return self
                .random_state
                .map(|state| i128::from(next_random_from_state(state)));
        }
        if name == "LINENO" {
            return self
                .env_vars
                .get("__RUBASH_CURRENT_LINE")
                .and_then(|line| line.parse::<i128>().ok())
                .or(Some(1));
        }

        let value = self
            .env_vars
            .get(name)
            .cloned()
            .or_else(|| std::env::var(name).ok())
            .unwrap_or_default();
        self.evaluate_variable_text(name, &value)
    }

    fn evaluate_variable_text(&mut self, resolving_name: &str, value: &str) -> Option<i128> {
        if self
            .resolving
            .iter()
            .any(|resolving| resolving == resolving_name)
        {
            return None;
        }

        let value = value.trim();
        if value.is_empty() {
            return Some(0);
        }
        if let Ok(number) = value.parse::<i128>() {
            return Some(bash_arith(number));
        }

        let mut resolving = self.resolving.clone();
        resolving.push(resolving_name.to_string());
        let mut parser = ConditionalArithParser {
            input: value.as_bytes(),
            pos: 0,
            env_vars: self.env_vars,
            resolving,
            random_state: self.random_state,
        };
        let value = parser.parse_comma()?;
        parser.skip_ws();
        (parser.pos == parser.input.len()).then_some(value)
    }

    fn update_lvalue(&mut self, lvalue: &ArithLValue, delta: i128, prefix: bool) -> Option<i128> {
        let current = self.lvalue_value(lvalue)?;
        let updated = bash_arith(current + delta);
        self.set_lvalue(lvalue, updated);
        Some(if prefix { updated } else { current })
    }

    fn assign_lvalue(&mut self, lvalue: &ArithLValue, op: &str, rhs: i128) -> Option<i128> {
        if op == "=" {
            self.set_lvalue(lvalue, rhs);
            return Some(rhs);
        }
        let current = self.lvalue_value(lvalue)?;
        let value = match op {
            "+=" => bash_arith(current + rhs),
            "-=" => bash_arith(current - rhs),
            "*=" => bash_arith(current * rhs),
            "**=" => checked_arithmetic_pow(current, rhs)?,
            "<<=" => bash_arith((current as i64).wrapping_shl(u32::try_from(rhs).ok()?) as i128),
            ">>=" => bash_arith((current as i64).wrapping_shr(u32::try_from(rhs).ok()?) as i128),
            "&=" => bash_arith(current & rhs),
            "^=" => bash_arith(current ^ rhs),
            "|=" => bash_arith(current | rhs),
            "/=" if rhs != 0 => bash_arith((current as i64).wrapping_div(rhs as i64) as i128),
            "%=" if rhs != 0 => {
                if current == i128::from(i64::MIN) && rhs == -1 {
                    return None;
                }
                current % rhs
            }
            "/=" | "%=" => return None,
            _ => return None,
        };
        self.set_lvalue(lvalue, value);
        Some(value)
    }

    fn set_lvalue(&mut self, lvalue: &ArithLValue, value: i128) {
        match lvalue {
            ArithLValue::Scalar(name) => self.set_variable(name, value),
            ArithLValue::Indexed { name, index } => self.set_array_element(name, *index, value),
            ArithLValue::Assoc { name, key } => self.set_assoc_element(name, key, value),
        }
    }

    fn set_variable(&mut self, name: &str, value: i128) {
        if is_noassign_bash_array(name) {
            return;
        }
        let value = bash_arith(value).to_string();
        if name == "RANDOM" {
            if let Some(state) = self.random_state {
                state.set(value.parse::<u32>().unwrap_or(0));
            }
        }
        self.env_vars.insert(name.to_string(), value.clone());
        set_process_env(name, value);
    }

    fn set_array_element(&mut self, name: &str, index: i128, value: i128) {
        if is_noassign_bash_array(name) {
            return;
        }
        let mut entries = self
            .env_vars
            .get(name)
            .map(|value| indexed_array_entries(value))
            .unwrap_or_default();
        let index = if index < 0 {
            let storage = format_indexed_array_storage(entries.clone());
            let Some(index) = resolve_indexed_array_subscript(&storage, index) else {
                return;
            };
            index
        } else {
            let Ok(index) = usize::try_from(index) else {
                return;
            };
            index
        };
        entries.insert(index, value.to_string());
        let value = format_indexed_array_storage(entries);
        self.env_vars.insert(name.to_string(), value);
        mark_env_name(self.env_vars, ARRAY_VARS, name);
    }

    fn set_assoc_element(&mut self, name: &str, key: &str, value: i128) {
        let mut entries = self
            .env_vars
            .get(name)
            .map(|value| assoc_entries(value))
            .unwrap_or_default();
        let value = value.to_string();
        if let Some((_, existing)) = entries.iter_mut().find(|(entry_key, _)| entry_key == key) {
            *existing = value;
        } else {
            entries.push((key.to_string(), value));
        }
        self.env_vars
            .insert(name.to_string(), format_assoc_storage(entries));
        mark_env_name(self.env_vars, ASSOC_VARS, name);
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(|ch| ch.is_ascii_whitespace()) {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn consume(&mut self, value: &str) -> bool {
        if self.input[self.pos..].starts_with(value.as_bytes()) {
            self.pos += value.len();
            true
        } else {
            false
        }
    }

    fn starts_with(&self, value: &str) -> bool {
        self.input[self.pos..].starts_with(value.as_bytes())
    }

    fn skip_arithmetic_rhs(&mut self, boundaries: &[&str]) {
        let mut depth = 0usize;
        while self.pos < self.input.len() {
            if depth == 0
                && boundaries
                    .iter()
                    .any(|boundary| self.input[self.pos..].starts_with(boundary.as_bytes()))
            {
                return;
            }

            match self.input[self.pos] {
                b'(' => {
                    depth += 1;
                    self.pos += 1;
                }
                b')' => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                    self.pos += 1;
                }
                _ => self.pos += 1,
            }
        }
    }

    fn skip_arithmetic_conditional_branch(&mut self, boundaries: &[&str]) {
        let mut depth = 0usize;
        let mut ternary_depth = 0usize;
        while self.pos < self.input.len() {
            if depth == 0
                && ternary_depth == 0
                && boundaries
                    .iter()
                    .any(|boundary| self.input[self.pos..].starts_with(boundary.as_bytes()))
            {
                return;
            }

            match self.input[self.pos] {
                b'(' => {
                    depth += 1;
                    self.pos += 1;
                }
                b')' => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                    self.pos += 1;
                }
                b'?' if depth == 0 => {
                    ternary_depth += 1;
                    self.pos += 1;
                }
                b':' if depth == 0 && ternary_depth > 0 => {
                    ternary_depth -= 1;
                    self.pos += 1;
                }
                _ => self.pos += 1,
            }
        }
    }
}
