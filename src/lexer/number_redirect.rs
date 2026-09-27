use super::scanner::Lexer;
use super::token::{Token, TokenKind};

impl<'a> Lexer<'a> {
    pub(super) fn finish_number_token(&mut self, start: usize) -> Token {
        while self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
            self.advance();
        }

        // GNU parse.y:5725-5738 read_token_word() (got_token): an all-digit
        // token ending right before `<`/`>` is a NUMBER redirection prefix
        // only when valid_number() succeeds AND the value fits in `int`
        // (`(int)lvalue == lvalue`; valid_number itself is strtoimax with an
        // ERANGE check, general.c:248). An int-range miss or intmax overflow
        // falls through and returns the digits as a plain WORD, so `<`/`>` is
        // lexed as a normal operator: `1111111111111111111111</dev/stdin` is
        // the command `1111111111111111111111` with an input redirection and
        // fails with "command not found" (rubash#197), never a silent fd
        // redirect. Single-digit tokens always fit, so this check is a no-op
        // for the scanner's single-digit dispatch arms.
        if matches!(self.peek(), Some('<' | '>')) {
            let digits = self.slice(start);
            let fits_int = digits
                .parse::<i64>()
                .ok()
                .and_then(|v| i32::try_from(v).ok())
                .is_some();
            if !fits_int {
                return self.finish_word_token(start, true);
            }
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
        // GNU parse.y:2626 read_token hands `((` to parse_dparen
        // (parse.y:3517+), which consumes the arithmetic body through
        // parse_matched_pair as raw text — so `2<<3` inside `(( ))` is a
        // shift of the number 2, never an fd-prefixed here-document
        // (rubash#181). Emit just the number word and let the guarded `<`
        // branch produce the `<<` operator token.
        if self.peek() == Some('<')
            && self.input.as_bytes().get(self.position + 1) == Some(&b'<')
            && self.inside_arithmetic_command()
        {
            return self.finish_word_token(start, true);
        }
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
