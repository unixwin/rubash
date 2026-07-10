use super::scanner::Lexer;
use super::token::{Token, TokenKind};

impl<'a> Lexer<'a> {
    pub(super) fn finish_number_token(&mut self, start: usize) -> Token {
        while self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
            self.advance();
        }

        match self.peek() {
            Some('>') => self.finish_number_output_redirect(start),
            Some('<') => self.finish_number_input_redirect(start),
            _ => self.finish_word_token(start, true),
        }
    }

    fn finish_number_output_redirect(&mut self, start: usize) -> Token {
        self.advance();
        let kind = if self.peek() == Some('>') {
            self.advance();
            if self.slice(start) == "2>>" {
                TokenKind::RedirectErrAppend
            } else {
                TokenKind::Append
            }
        } else {
            if matches!(self.peek(), Some('&' | '|')) {
                self.advance();
            }
            if self.slice(start).starts_with("2>") && self.slice(start).len() <= 3 {
                TokenKind::RedirectErr
            } else {
                TokenKind::RedirectOut
            }
        };
        Token::new(kind, self.slice(start), start)
    }

    fn finish_number_input_redirect(&mut self, start: usize) -> Token {
        self.advance();
        match self.peek() {
            Some('>') => {
                self.advance();
                Token::new(TokenKind::RedirectOut, self.slice(start), start)
            }
            Some('&') => {
                self.advance();
                Token::new(TokenKind::RedirectIn, self.slice(start), start)
            }
            Some('<') => {
                self.advance();
                if matches!(self.peek(), Some('<' | '-')) {
                    self.advance();
                }
                if self.slice(start).ends_with("<<<") {
                    Token::new(TokenKind::HereString, self.slice(start), start)
                } else {
                    Token::new(TokenKind::HereDoc, self.slice(start), start)
                }
            }
            _ => Token::new(TokenKind::RedirectIn, self.slice(start), start),
        }
    }
}
