use super::ConditionalArithParser;

impl ConditionalArithParser<'_> {
    pub(in crate::executor::arithmetic) fn skip_ws(&mut self) {
        while self.peek().is_some_and(|ch| ch.is_ascii_whitespace()) {
            self.pos += 1;
        }
    }

    pub(super) fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    pub(super) fn consume(&mut self, value: &str) -> bool {
        if self.input[self.pos..].starts_with(value.as_bytes()) {
            self.pos += value.len();
            true
        } else {
            false
        }
    }

    pub(super) fn starts_with(&self, value: &str) -> bool {
        self.input[self.pos..].starts_with(value.as_bytes())
    }

    pub(super) fn skip_arithmetic_rhs(&mut self, boundaries: &[&str]) {
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

    pub(super) fn skip_arithmetic_conditional_branch(&mut self, boundaries: &[&str]) {
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
