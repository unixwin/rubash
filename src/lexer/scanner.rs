use std::str::from_utf8;

use super::classification::{is_brace_expansion, is_word_delimiter};
use super::token::{Token, TokenKind};

pub(super) struct Lexer<'a> {
    pub(super) input: &'a [u8],
    pub(super) position: usize,
}

impl<'a> Lexer<'a> {
    pub(super) fn new(input: &'a str) -> Self {
        Self {
            input: input.as_bytes(),
            position: 0,
        }
    }

    #[inline]
    pub(super) fn at_end(&self) -> bool {
        self.position >= self.input.len()
    }

    #[inline]
    pub(super) fn peek(&self) -> Option<char> {
        if self.at_end() {
            None
        } else {
            from_utf8(&self.input[self.position..]).ok()?.chars().next()
        }
    }

    #[inline]
    pub(super) fn peek_after(&self, offset: usize) -> Option<char> {
        from_utf8(&self.input[self.position..])
            .ok()?
            .chars()
            .nth(offset)
    }

    #[inline]
    pub(super) fn advance(&mut self) -> Option<char> {
        if self.at_end() {
            None
        } else {
            let c = from_utf8(&self.input[self.position..])
                .ok()?
                .chars()
                .next()?;
            self.position += c.len_utf8();
            Some(c)
        }
    }

    pub(super) fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c == ' ' || c == '\t' {
                self.advance();
            } else {
                break;
            }
        }
    }

    pub(super) fn slice(&self, start: usize) -> &str {
        let end = self.position.min(self.input.len());
        from_utf8(&self.input[start..end]).unwrap_or("")
    }

    pub(super) fn next_token(&mut self) -> Option<Token> {
        self.skip_ws();
        if self.at_end() {
            return Some(Token::new(TokenKind::Eof, "", self.position));
        }

        let start = self.position;
        let c = self.advance()?;

        match c {
            '\n' => Some(Token::new(TokenKind::Semicolon, ";", start)),
            '|' => {
                if self.peek() == Some('|') {
                    self.advance();
                    Some(Token::new(TokenKind::Or, "||", start))
                } else if self.peek() == Some('&') {
                    self.advance();
                    Some(Token::new(TokenKind::PipeErr, "|&", start))
                } else {
                    Some(Token::new(TokenKind::Pipe, "|", start))
                }
            }
            '&' => {
                if self.peek() == Some('&') {
                    self.advance();
                    Some(Token::new(TokenKind::And, "&&", start))
                } else if self.peek() == Some('>') {
                    self.advance();
                    if self.peek() == Some('>') {
                        self.advance();
                        Some(Token::new(TokenKind::Append, "&>>", start))
                    } else {
                        Some(Token::new(TokenKind::RedirectOut, "&>", start))
                    }
                } else if self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
                    self.skip_word();
                    Some(Token::new(TokenKind::Word, self.slice(start), start))
                } else {
                    Some(Token::new(TokenKind::Background, "&", start))
                }
            }
            '(' | ')' => Some(Token::new(TokenKind::Keyword, self.slice(start), start)),
            '!' => {
                if self.peek() == Some('=') {
                    self.skip_word();
                    Some(Token::new(TokenKind::Word, self.slice(start), start))
                } else if self.peek() == Some('(') {
                    Some(self.finish_word_token(start, false))
                } else {
                    Some(Token::new(TokenKind::Keyword, "!", start))
                }
            }
            ';' => {
                if self.peek() == Some(';') {
                    self.advance();
                    if self.peek() == Some('&') {
                        self.advance();
                        Some(Token::new(TokenKind::Word, ";;&", start))
                    } else {
                        Some(Token::new(TokenKind::Word, ";;", start))
                    }
                } else if self.peek() == Some('&') {
                    self.advance();
                    Some(Token::new(TokenKind::Word, ";&", start))
                } else {
                    Some(Token::new(TokenKind::Semicolon, ";", start))
                }
            }
            '<' => match self.peek() {
                Some('<') => {
                    self.advance();
                    if self.peek() == Some('<') {
                        self.advance();
                        Some(Token::new(TokenKind::HereString, "<<<", start))
                    } else if self.peek() == Some('-') {
                        self.advance();
                        Some(Token::new(TokenKind::HereDoc, "<<-", start))
                    } else {
                        Some(Token::new(TokenKind::HereDoc, "<<", start))
                    }
                }
                Some('>') => {
                    self.advance();
                    Some(Token::new(TokenKind::RedirectOut, "<>", start))
                }
                Some('&') => {
                    self.advance();
                    Some(Token::new(TokenKind::RedirectIn, "<&", start))
                }
                _ => Some(Token::new(TokenKind::RedirectIn, "<", start)),
            },
            '>' => {
                if self.peek() == Some('>') {
                    self.advance();
                    Some(Token::new(TokenKind::Append, ">>", start))
                } else if self.peek() == Some('&') {
                    self.advance();
                    Some(Token::new(TokenKind::RedirectOut, ">&", start))
                } else if self.peek() == Some('|') {
                    self.advance();
                    Some(Token::new(TokenKind::RedirectOut, ">|", start))
                } else {
                    Some(Token::new(TokenKind::RedirectOut, ">", start))
                }
            }
            '0'..='9' if self.peek().is_some_and(|ch| ch.is_ascii_digit()) => {
                Some(self.finish_number_token(start))
            }
            '0'..='9' if c != '2' && self.peek() == Some('>') => {
                self.advance();
                if self.peek() == Some('>') {
                    self.advance();
                    Some(Token::new(TokenKind::Append, self.slice(start), start))
                } else if self.peek() == Some('&') {
                    self.advance();
                    Some(Token::new(TokenKind::RedirectOut, self.slice(start), start))
                } else if self.peek() == Some('|') {
                    self.advance();
                    Some(Token::new(TokenKind::RedirectOut, self.slice(start), start))
                } else {
                    Some(Token::new(TokenKind::RedirectOut, self.slice(start), start))
                }
            }
            '0'..='9' if c != '2' && self.peek() == Some('<') => {
                Some(self.finish_prefixed_input_redirect(start))
            }
            '2' => {
                if self.peek() == Some('>') {
                    self.advance();
                    if self.peek() == Some('>') {
                        self.advance();
                        Some(Token::new(TokenKind::RedirectErrAppend, "2>>", start))
                    } else if self.peek() == Some('&') {
                        self.advance();
                        Some(Token::new(TokenKind::RedirectErr, "2>&", start))
                    } else if self.peek() == Some('|') {
                        self.advance();
                        Some(Token::new(TokenKind::RedirectErr, "2>|", start))
                    } else {
                        Some(Token::new(TokenKind::RedirectErr, "2>", start))
                    }
                } else if self.peek() == Some('<') {
                    Some(self.finish_prefixed_input_redirect(start))
                } else {
                    self.skip_word();
                    Some(Token::new(TokenKind::Word, self.slice(start), start))
                }
            }
            '#' => {
                while self.advance().is_some_and(|ch| ch != '\n') {}
                self.next_token()
            }
            '$' => match self.peek() {
                Some('\'') => {
                    self.advance();
                    self.skip_ansi_c_single();
                    Some(self.finish_word_token(start, false))
                }
                Some('(') => {
                    self.advance();
                    self.skip_cmd_subst();
                    if self.peek().is_some_and(|ch| !is_word_delimiter(ch)) {
                        return Some(self.finish_word_token(start, false));
                    }
                    Some(Token::new(
                        TokenKind::CommandSubst,
                        self.slice(start),
                        start,
                    ))
                }
                Some('{') => {
                    self.advance();
                    self.skip_braced();
                    if self.peek().is_some_and(|ch| !is_word_delimiter(ch)) {
                        return Some(self.finish_word_token(start, false));
                    }
                    Some(Token::new(TokenKind::Variable, self.slice(start), start))
                }
                _ => {
                    let pos = self.position;
                    self.skip_word();
                    Some(Token::new(
                        TokenKind::Variable,
                        &format!("${}", self.slice(pos)),
                        start,
                    ))
                }
            },
            '`' => {
                self.skip_backtick();
                if self.peek().is_some_and(|ch| !is_word_delimiter(ch)) {
                    return Some(self.finish_word_token(start, false));
                }
                Some(Token::new(
                    TokenKind::CommandSubst,
                    self.slice(start),
                    start,
                ))
            }
            '\'' => {
                self.skip_single();
                Some(self.finish_word_token(start, false))
            }
            '"' => {
                self.skip_double();
                Some(self.finish_word_token(start, false))
            }
            '\\' => {
                self.advance();
                Some(self.finish_word_token(start, false))
            }
            '{' => {
                self.skip_brace();
                if self.peek().is_some_and(|ch| !is_word_delimiter(ch)) {
                    return Some(self.finish_word_token(start, false));
                }
                let v = self.slice(start);
                let kind = if is_brace_expansion(v) {
                    TokenKind::BraceExpand
                } else {
                    TokenKind::Keyword
                };
                Some(Token::new(kind, v, start))
            }
            '}' => Some(Token::new(TokenKind::Keyword, "}", start)),
            _ => Some(self.finish_word_token(start, true)),
        }
    }

    fn finish_prefixed_input_redirect(&mut self, start: usize) -> Token {
        if self.peek_after(1) == Some('<') {
            return self.finish_number_token(start);
        }

        self.advance();
        if matches!(self.peek(), Some('>' | '&')) {
            self.advance();
        }
        let kind = if self.slice(start).ends_with("<>") {
            TokenKind::RedirectOut
        } else {
            TokenKind::RedirectIn
        };
        Token::new(kind, self.slice(start), start)
    }
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Token;
    fn next(&mut self) -> Option<Self::Item> {
        self.next_token()
    }
}
