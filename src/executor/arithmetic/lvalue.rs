use super::{ArithLValue, ConditionalArithParser};
use crate::executor::arithmetic::assignment_operator_at;
use crate::executor::{
    is_marked_var, is_shell_name, is_shell_name_char, is_shell_name_start, strip_matching_quotes,
    ASSOC_VARS, NAMEREF_VARS,
};
use std::collections::HashSet;

impl ConditionalArithParser<'_> {
    pub(super) fn parse_lvalue(&mut self) -> Option<ArithLValue> {
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

    pub(super) fn resolved_lvalue_name(&self, name: &str) -> String {
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

    pub(super) fn parse_assoc_subscript(&mut self) -> Option<String> {
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

    pub(super) fn expand_assoc_subscript_key(&self, key: &str) -> String {
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

    pub(super) fn consume_assignment_operator(&mut self) -> Option<&'static str> {
        let op = assignment_operator_at(self.input, self.pos)?;
        self.pos += op.len();
        Some(op)
    }
}
