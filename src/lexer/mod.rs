//! Lexer Module - Bash Tokenizer
//!
//! Transforms raw input strings into tokens for the parser.

use std::str::from_utf8;

/// Token types for bash
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Word, Pipe, Semicolon, RedirectOut, RedirectIn, Append,
    RedirectErr, RedirectErrAppend, HereDoc, And, Or,
    Keyword, Variable, Assignment, CommandSubst, BraceExpand, Eof,
}

/// A single token with its kind, value, and position
#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub value: String,
    pub position: usize,
}

impl Token {
    pub fn new(kind: TokenKind, value: &str, position: usize) -> Self {
        Self { kind, value: value.to_string(), position }
    }
}

fn is_keyword(word: &str) -> bool {
    matches!(word, "if" | "then" | "else" | "elif" | "fi"
        | "while" | "do" | "done" | "until" | "for" | "case" | "esac" | "in"
        | "function" | "select" | "time" | "coproc")
}

fn is_assignment(word: &str) -> bool {
    let Some(pos) = word.find('=') else { return false };
    let var_name = &word[..pos];
    !var_name.is_empty()
        && var_name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && var_name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_brace_expansion(word: &str) -> bool {
    word.starts_with('{') && word.ends_with('}') && word.len() >= 3
        && (word[1..word.len()-1].contains("..") || word.contains(','))
}

/// Tokenize a string into tokens
pub fn tokenize(input: &str) -> Vec<Token> {
    let lexer = Lexer::new(input);
    let mut tokens = Vec::new();
    for token in lexer {
        if token.kind == TokenKind::Eof { break; }
        tokens.push(token);
    }
    tokens
}

pub struct Lexer<'a> {
    input: &'a [u8],
    position: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        Self { input: input.as_bytes(), position: 0 }
    }

    #[inline] fn at_end(&self) -> bool { self.position >= self.input.len() }

    #[inline] fn peek(&self) -> Option<char> {
        if self.at_end() { None } else { from_utf8(&self.input[self.position..]).ok()?.chars().next() }
    }

    #[inline] fn advance(&mut self) -> Option<char> {
        if self.at_end() { None } else {
            let c = from_utf8(&self.input[self.position..]).ok()?.chars().next()?;
            self.position += c.len_utf8();
            Some(c)
        }
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c == ' ' || c == '\t' || c == '\n' { self.advance(); } else { break; }
        }
    }

    fn slice(&self, start: usize) -> &str {
        let end = self.position.min(self.input.len());
        from_utf8(&self.input[start..end]).unwrap_or("")
    }

    fn next_token(&mut self) -> Option<Token> {
        self.skip_ws();
        if self.at_end() { return Some(Token::new(TokenKind::Eof, "", self.position)); }

        let start = self.position;
        let c = self.advance()?;

        match c {
            '|' => {
                if self.peek() == Some('|') { self.advance(); Some(Token::new(TokenKind::Or, "||", start)) }
                else { Some(Token::new(TokenKind::Pipe, "|", start)) }
            }
            '&' => {
                if self.peek() == Some('&') { self.advance(); Some(Token::new(TokenKind::And, "&&", start)) }
                else { self.skip_word(); Some(Token::new(TokenKind::Word, self.slice(start), start)) }
            }
            ';' => {
                if self.peek() == Some(';') { self.advance(); Some(Token::new(TokenKind::Word, ";;", start)) }
                else { Some(Token::new(TokenKind::Semicolon, ";", start)) }
            }
            '<' => {
                match self.peek() {
                    Some('<') => { self.advance(); Some(Token::new(TokenKind::HereDoc, "<<", start)) }
                    Some('>') => { self.advance(); Some(Token::new(TokenKind::RedirectOut, "<>", start)) }
                    _ => Some(Token::new(TokenKind::RedirectIn, "<", start))
                }
            }
            '>' => {
                if self.peek() == Some('>') { self.advance(); Some(Token::new(TokenKind::Append, ">>", start)) }
                else { Some(Token::new(TokenKind::RedirectOut, ">", start)) }
            }
            '2' => {
                if self.peek() == Some('>') {
                    self.advance();
                    if self.peek() == Some('>') { self.advance(); Some(Token::new(TokenKind::RedirectErrAppend, "2>>", start)) }
                    else { Some(Token::new(TokenKind::RedirectErr, "2>", start)) }
                } else {
                    self.skip_word(); Some(Token::new(TokenKind::Word, self.slice(start), start))
                }
            }
            '#' => { while self.advance().is_some_and(|ch| ch != '\n') {} self.next_token() }
            '$' => {
                match self.peek() {
                    Some('(') => { self.advance(); self.skip_cmd_subst(); Some(Token::new(TokenKind::CommandSubst, self.slice(start), start)) }
                    Some('{') => { self.advance(); self.skip_braced(); Some(Token::new(TokenKind::Variable, self.slice(start), start)) }
                    _ => { let pos = self.position; self.skip_word(); Some(Token::new(TokenKind::Variable, &format!("${}", self.slice(pos)), start)) }
                }
            }
            '`' => { self.skip_backtick(); Some(Token::new(TokenKind::CommandSubst, self.slice(start), start)) }
            '\'' => { self.skip_single(); Some(self.finish_word_token(start, false)) }
            '"' => { self.skip_double(); Some(self.finish_word_token(start, false)) }
            '\\' => { self.advance(); Some(self.finish_word_token(start, false)) }
            '{' => { self.skip_brace(); let v = self.slice(start); let kind = if is_brace_expansion(v) { TokenKind::BraceExpand } else { TokenKind::Keyword }; Some(Token::new(kind, v, start)) }
            '}' => Some(Token::new(TokenKind::Keyword, "}", start)),
            _ => Some(self.finish_word_token(start, true))
        }
    }

    fn finish_word_token(&mut self, start: usize, allow_keyword: bool) -> Token {
        self.skip_word();
        let raw = self.slice(start);
        let value = remove_shell_quotes(raw);
        let kind = if allow_keyword && is_keyword(raw) {
            TokenKind::Keyword
        } else if is_assignment(&value) {
            TokenKind::Assignment
        } else {
            TokenKind::Word
        };
        Token::new(kind, &value, start)
    }

    fn skip_word(&mut self) {
        while let Some(c) = self.peek() {
            if " \t\n|&;<>#$`{}".contains(c) { break; }
            match c {
                '\'' => {
                    self.advance();
                    self.skip_single();
                }
                '"' => {
                    self.advance();
                    self.skip_double();
                }
                '\\' => {
                    self.advance();
                    self.advance();
                }
                _ => {
                    self.advance();
                }
            }
        }
    }

    fn skip_cmd_subst(&mut self) {
        let mut depth = 1;
        while let Some(c) = self.advance() {
            match c {
                '(' => depth += 1,
                ')' => { depth -= 1; if depth == 0 { break; } }
                '\'' => self.skip_single(),
                '"' => self.skip_double(),
                _ => {}
            }
        }
    }

    fn skip_backtick(&mut self) { while let Some(c) = self.advance() { if c == '`' { break; } else if c == '\\' { self.advance(); } } }
    fn skip_single(&mut self) { while let Some(c) = self.advance() { if c == '\'' { break; } } }
    fn skip_double(&mut self) { while let Some(c) = self.advance() { if c == '"' { break; } else if c == '\\' { self.advance(); } } }
    fn skip_braced(&mut self) { while let Some(c) = self.advance() { if c == '}' { break; } } }
    fn skip_brace(&mut self) { while let Some(c) = self.advance() { if c == '}' { break; } } }
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Token;
    fn next(&mut self) -> Option<Self::Item> { self.next_token() }
}

#[cfg(test)] mod unit_tests {
    use super::*;

    #[test] fn test_tokenize_simple() {
        let tokens = tokenize("ls -la");
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0].value, "ls");
    }

    #[test] fn test_tokenize_empty() { assert!(tokenize("").is_empty()); }

    #[test] fn test_comment_skip() {
        let tokens = tokenize("ls # comment");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].value, "ls");
    }
}

fn remove_shell_quotes(raw: &str) -> String {
    let mut out = String::new();
    let mut chars = raw.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\'' => {
                for quoted in chars.by_ref() {
                    if quoted == '\'' {
                        break;
                    }
                    out.push(quoted);
                }
            }
            '"' => {
                while let Some(quoted) = chars.next() {
                    match quoted {
                        '"' => break,
                        '\\' => {
                            if let Some(escaped @ ('\\' | '"' | '$' | '`' | '\n')) = chars.peek().copied() {
                                chars.next();
                                if escaped != '\n' {
                                    out.push(escaped);
                                }
                            } else {
                                out.push('\\');
                            }
                        }
                        _ => out.push(quoted),
                    }
                }
            }
            '\\' => {
                if let Some(escaped) = chars.next() {
                    out.push(escaped);
                }
            }
            _ => out.push(ch),
        }
    }

    out
}
