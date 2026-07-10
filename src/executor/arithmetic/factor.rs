use super::ConditionalArithParser;
use crate::executor::arithmetic::{arithmetic_digit_value, bash_arith, parse_arithmetic_digits};
use crate::executor::{is_shell_name_char, is_shell_name_start};

impl ConditionalArithParser<'_> {
    pub(super) fn parse_factor(&mut self) -> Option<i128> {
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

    pub(super) fn parse_number(&mut self) -> Option<i128> {
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

    pub(super) fn parse_dollar_variable(&mut self) -> Option<i128> {
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

    pub(super) fn parse_variable(&mut self) -> Option<i128> {
        let lvalue = self.parse_lvalue()?;
        if self.consume("++") {
            return self.update_lvalue(&lvalue, 1, false);
        }
        if self.consume("--") {
            return self.update_lvalue(&lvalue, -1, false);
        }
        self.lvalue_value(&lvalue)
    }
}
