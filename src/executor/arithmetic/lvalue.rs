use super::{ArithLValue, ConditionalArithParser};
use crate::executor::arithmetic::{
    assignment_operator_at, eval_mutable_arith_value_with_random, strip_arith_double_quotes,
};
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

        let index = {
            self.skip_ws();
            if self.consume("]") {
                0
            } else if self.peek() == Some(b'"') || self.peek() == Some(b'\'') {
                let expression = self.collect_quoted_index_expression()?;
                let expression = strip_arith_double_quotes(&expression);
                if expression.is_empty() {
                    self.error_category =
                        Some(super::super::ArithmeticErrorCategory::EmptyArraySubscript);
                    return None;
                }
                eval_mutable_arith_value_with_random(&expression, self.env_vars, self.random_state)
                    .0?
            } else {
                let index = self.parse_comma()?;
                self.skip_ws();
                if !self.consume("]") {
                    return None;
                }
                index
            }
        };
        Some(ArithLValue::Indexed {
            name: resolved_name,
            index,
        })
    }

    fn collect_quoted_index_expression(&mut self) -> Option<String> {
        let start = self.pos;
        let mut bracket_depth = 0usize;
        let mut single = false;
        let mut double = false;
        let mut escaped = false;
        while let Some(ch) = self.peek() {
            self.pos += 1;
            if escaped {
                escaped = false;
                continue;
            }
            if ch == b'\\' && !single {
                escaped = true;
                continue;
            }
            match ch {
                b'\'' if !double => single = !single,
                b'"' if !single => double = !double,
                b'[' if !single && !double => bracket_depth += 1,
                b']' if !single && !double && bracket_depth > 0 => bracket_depth -= 1,
                b']' if !single && !double => {
                    let expression = std::str::from_utf8(&self.input[start..self.pos - 1])
                        .ok()?
                        .to_string();
                    return Some(expression);
                }
                _ => {}
            }
        }
        None
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
                    // The raw subscript is data: GNU expand_subscript_string
                    // keeps IFS whitespace that surrounds or makes up the key
                    // (`k=$'\t'; A[$k]=2` keys on the tab, and `A[ $k ]` keys
                    // on ` x `), so it must not be trimmed away.
                    let key = std::str::from_utf8(&self.input[start..self.pos])
                        .ok()?
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
        // A wholly single-quoted subscript is literal data: GNU's
        // expand_subscript_string removes the quotes but runs no expansion
        // inside a single-quoted span, so `A['$var']` keys on the text `$var`
        // and `A['a b']` keys on `a b`.
        if let Some(literal) = wholly_single_quoted_literal(key) {
            return literal;
        }

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

        // Quote removal only: an associative subscript is a string key, so
        // surrounding IFS whitespace is data, not padding (GNU keeps it).
        strip_matching_quotes(&output).to_string()
    }

    pub(super) fn consume_assignment_operator(&mut self) -> Option<&'static str> {
        let op = assignment_operator_at(self.input, self.pos)?;
        self.pos += op.len();
        Some(op)
    }
}

/// The concatenated contents of `text` when it is covered entirely by
/// single-quoted spans (`'a b'`, `'a''b'`); `None` when any character sits
/// outside a single-quoted span, in which case the subscript still has to be
/// expanded.
fn wholly_single_quoted_literal(text: &str) -> Option<String> {
    let mut out = String::new();
    let mut rest = text;
    let mut saw_span = false;
    while !rest.is_empty() {
        let inner = rest.strip_prefix('\'')?;
        let end = inner.find('\'')?;
        out.push_str(&inner[..end]);
        rest = &inner[end + 1..];
        saw_span = true;
    }
    saw_span.then_some(out)
}
