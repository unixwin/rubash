use super::classification::{is_brace_expansion, is_word_delimiter};
use super::quotes::normalize_backtick_command_substitution;
use super::token::{Token, TokenKind};

pub(super) struct Lexer<'a> {
    pub(super) input: &'a str,
    pub(super) position: usize,
    /// POSIX parse mode (Austin Group Interp 221): single quotes inside a
    /// double-quoted `${...}` are literal, so `}` closes the expansion.
    pub(super) posix: bool,
}

impl<'a> Lexer<'a> {
    pub(super) fn new(input: &'a str, posix: bool) -> Self {
        Self {
            input,
            position: 0,
            posix,
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
            self.input[self.position..].chars().next()
        }
    }

    #[inline]
    pub(super) fn peek_after(&self, offset: usize) -> Option<char> {
        self.input[self.position..].chars().nth(offset)
    }

    #[inline]
    pub(super) fn advance(&mut self) -> Option<char> {
        if self.at_end() {
            None
        } else {
            let c = self.input[self.position..].chars().next()?;
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
        &self.input[start..end]
    }

    pub(super) fn next_token(&mut self) -> Option<Token> {
        self.skip_ws();
        if self.at_end() {
            return Some(Token::new(TokenKind::Eof, "", self.position));
        }

        let start = self.position;
        let c = self.advance()?;

        match c {
            '\n' => {
                let mut token = Token::new(TokenKind::Semicolon, ";", start);
                token.line_break = true;
                Some(token)
            }
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
                    self.skip_word_at(start);
                    Some(Token::new(TokenKind::Word, self.slice(start), start))
                } else {
                    Some(Token::new(TokenKind::Background, "&", start))
                }
            }
            '(' | ')' => Some(Token::new(TokenKind::Keyword, self.slice(start), start)),
            '!' => {
                if self.peek() == Some('=') {
                    self.skip_word_at(start);
                    Some(Token::new(TokenKind::Word, self.slice(start), start))
                } else if self.peek() == Some('(') {
                    Some(self.finish_word_token(start, false))
                } else if self
                    .peek()
                    .is_some_and(|ch| !is_word_delimiter(ch) && !ch.is_whitespace())
                {
                    // `!!`, `!2`, `!$`, history word designators and `!foo` are
                    // words when not isolated as the `!` pipeline negation
                    // keyword. Splitting `!!` into two `!` keywords makes
                    // `eval echo '!!'` print `! !` (histexp).
                    self.skip_word_at(start);
                    Some(Token::new(TokenKind::Word, self.slice(start), start))
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
                    self.skip_word_at(start);
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
                Some('"') => {
                    self.advance();
                    self.skip_double();
                    Some(self.finish_word_token(start, false))
                }
                Some('(') => {
                    self.advance();
                    if self.peek() == Some('(') {
                        self.advance();
                        self.skip_arith_paren();
                    } else {
                        self.skip_cmd_subst();
                    }
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
                    self.skip_braced(false);
                    // A brace group adjacent to a closed parameter expansion
                    // continues the same word (parse.y read_token_word keeps
                    // scanning until a real metacharacter): "${a}{x,y}" is
                    // one word whose brace group then expands. A `}` right
                    // after the close is literal word text too (GNU probe:
                    // `echo ${x-d{}}` is the single word d{}; splitting it
                    // off made the trailing brace a standalone word).
                    if self
                        .peek()
                        .is_some_and(|ch| !is_word_delimiter(ch) || ch == '{' || ch == '}')
                    {
                        return Some(self.finish_word_token(start, false));
                    }
                    Some(Token::new(TokenKind::Variable, self.slice(start), start))
                }
                Some('[') => {
                    self.advance();
                    self.skip_arith_bracket();
                    if self.peek().is_some_and(|ch| !is_word_delimiter(ch)) {
                        return Some(self.finish_word_token(start, false));
                    }
                    Some(Token::new(TokenKind::Word, self.slice(start), start))
                }
                _ => {
                    let pos = self.position;
                    self.skip_word_at(start);
                    if !is_simple_parameter_tail(self.slice(pos)) {
                        return Some(self.finish_word_token(start, false));
                    }
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
                let raw = self.slice(start);
                let value = normalize_backtick_command_substitution(raw);
                Some(Token::new_with_raw(
                    TokenKind::CommandSubst,
                    &value,
                    raw,
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
                if self.brace_group_contains_heredoc_operator() {
                    return Some(Token::new(TokenKind::Keyword, "{", start));
                }
                self.skip_brace();
                if self
                    .peek()
                    .is_some_and(|ch| !is_word_delimiter(ch) || ch == '{')
                {
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

    fn brace_group_contains_heredoc_operator(&self) -> bool {
        let chars = self.input[self.position..].chars().collect::<Vec<_>>();
        let mut index = 0usize;
        let mut depth = 1usize;
        let mut single = false;
        let mut double = false;
        let mut escaped = false;

        while index < chars.len() {
            let ch = chars[index];
            if escaped {
                escaped = false;
                index += 1;
                continue;
            }
            if ch == '\\' && !single {
                escaped = true;
                index += 1;
                continue;
            }
            if ch == '\'' && !double {
                single = !single;
                index += 1;
                continue;
            }
            if ch == '"' && !single {
                double = !double;
                index += 1;
                continue;
            }
            if single || double {
                index += 1;
                continue;
            }

            match ch {
                '{' => depth += 1,
                '}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return false;
                    }
                }
                '<' if chars.get(index + 1) == Some(&'<') && chars.get(index + 2) != Some(&'<') => {
                    return true;
                }
                _ => {}
            }
            index += 1;
        }

        false
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

fn is_simple_parameter_tail(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    if matches!(first, '?' | '$' | '!' | '@' | '*' | '#' | '-') {
        return chars.next().is_none();
    }

    if first.is_ascii_digit() {
        return chars.next().is_none();
    }

    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Token;
    fn next(&mut self) -> Option<Self::Item> {
        self.next_token()
    }
}
