//! Arithmetic expression parsing and evaluation.
//!
//! Provides parsing and evaluation of shell arithmetic expressions including
//! variables, arrays, assignments, and ternary conditionals.

mod parser;

use parser::ConditionalArithParser;
use std::cell::Cell;
use std::collections::HashMap;

use super::Executor;

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

pub(super) fn eval_conditional_arith_value(
    value: &str,
    env_vars: &HashMap<String, String>,
) -> Option<i128> {
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
