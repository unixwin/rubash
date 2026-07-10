use super::classification::{
    assignment_value_is_quoted, has_unquoted_assignment_equal, is_assignment, is_keyword,
    mark_quoted_assignment_value, quoted_literal_tilde,
};
use super::quotes::{remove_shell_quotes, remove_shell_quotes_outside_backticks};
use super::scanner::Lexer;
use super::token::{Token, TokenKind};

impl<'a> Lexer<'a> {
    pub(super) fn finish_word_token(&mut self, start: usize, allow_keyword: bool) -> Token {
        if self.word_so_far_ends_extglob_operator(start) && self.peek() == Some('(') {
            self.skip_extglob_group();
        }
        self.skip_word();
        let raw = self.slice(start);
        let value = if raw.contains('=') && raw.contains("$(") {
            // TODO(parse.y/subst.c): Preserve quotes inside `$()` while
            // assignment-word quote removal is still token-local.
            raw.to_string()
        } else if raw.contains('=') && raw.contains('`') {
            // TODO(parse.y/subst.c): Assignment-word quote removal must not
            // consume quotes inside command substitutions. Preserve the
            // backquote body for the substitution stage.
            remove_shell_quotes_outside_backticks(raw)
        } else {
            remove_shell_quotes(raw)
        };
        let kind = if allow_keyword && is_keyword(raw) {
            TokenKind::Keyword
        } else if is_assignment(&value) && has_unquoted_assignment_equal(raw) {
            TokenKind::Assignment
        } else {
            TokenKind::Word
        };
        let value = if quoted_literal_tilde(raw, &value) {
            // TODO(parse.y/subst.c): Preserve quote state as WORD_DESC flags.
            // This prevents quoted literal `~` from undergoing tilde
            // expansion before builtins like `printf %q` see it.
            format!("\x1b{value}")
        } else if kind == TokenKind::Assignment && assignment_value_is_quoted(raw) {
            // TODO(parse.y/subst.c): Replace this narrow quoted-RHS marker
            // with WORD_DESC quote flags. It lets assignment tilde expansion
            // distinguish `a=~/x` from `a="~/x"` without leaking syntax to
            // builtins.
            mark_quoted_assignment_value(&value)
        } else if kind == TokenKind::Word
            && is_assignment(&value)
            && assignment_value_is_quoted(raw)
        {
            // A fully quoted assignment-looking argument, such as
            // `"SHELL=~/bash"`, remains a normal word but its RHS quote state
            // still suppresses the assignment-word tilde pass.
            mark_quoted_assignment_value(&value)
        } else if raw.starts_with('"') && raw.ends_with('"') && raw.contains("${") {
            // TODO(parse.y/subst.c): Preserve full quote state on WORD_DESC
            // instead of a sentinel. This narrow marker lets expansion
            // distinguish "${v:-~}" from ${v:-~} for upstream tilde2.tests.
            format!("\x1d{value}")
        } else {
            value
        };
        Token::new(kind, &value, start)
    }

    pub(super) fn skip_word(&mut self) {
        let mut extglob_operator = false;
        while let Some(c) = self.peek() {
            if " \t\n|&;<>(){}".contains(c) {
                if c == '(' && extglob_operator {
                    self.skip_extglob_group();
                    extglob_operator = false;
                    continue;
                }
                break;
            }
            match c {
                '`' => {
                    // TODO(parse.y/subst.c): Command substitution is part of
                    // the surrounding word. Keeping it atomic is required for
                    // assignment words such as v=`echo x`.
                    self.advance();
                    self.skip_backtick();
                    extglob_operator = false;
                }
                '\'' => {
                    self.advance();
                    self.skip_single();
                    extglob_operator = false;
                }
                '"' => {
                    self.advance();
                    self.skip_double();
                    extglob_operator = false;
                }
                '\\' => {
                    self.advance();
                    self.advance();
                    extglob_operator = false;
                }
                '$' => {
                    self.advance();
                    match self.peek() {
                        Some('{') => {
                            self.advance();
                            self.skip_braced();
                        }
                        Some('(') => {
                            self.advance();
                            self.skip_cmd_subst();
                        }
                        Some('\'') => {
                            self.advance();
                            self.skip_ansi_c_single();
                        }
                        _ => {}
                    }
                    extglob_operator = false;
                }
                _ => {
                    self.advance();
                    extglob_operator = matches!(c, '@' | '*' | '+' | '?' | '!');
                }
            }
        }
    }

    fn word_so_far_ends_extglob_operator(&self, start: usize) -> bool {
        self.slice(start)
            .chars()
            .last()
            .is_some_and(|ch| matches!(ch, '@' | '*' | '+' | '?' | '!'))
    }

    fn skip_extglob_group(&mut self) {
        if self.peek() != Some('(') {
            return;
        }

        self.advance();
        let mut depth = 1usize;
        while let Some(c) = self.advance() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                '`' => self.skip_backtick(),
                '\'' => self.skip_single(),
                '"' => self.skip_double(),
                '\\' => {
                    self.advance();
                }
                _ => {}
            }
        }
    }
}
