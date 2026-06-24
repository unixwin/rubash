//! Lexer Module - Bash Tokenizer
//!
//! Transforms raw input strings into tokens for the parser.

use std::str::from_utf8;

/// Token types for bash
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Word,
    Pipe,
    Semicolon,
    RedirectOut,
    RedirectIn,
    Append,
    RedirectErr,
    RedirectErrAppend,
    HereDoc,
    HereString,
    Background,
    And,
    Or,
    Keyword,
    Variable,
    Assignment,
    CommandSubst,
    BraceExpand,
    HereDocBody,
    Eof,
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
        Self {
            kind,
            value: value.to_string(),
            position,
        }
    }
}

fn is_keyword(word: &str) -> bool {
    matches!(
        word,
        "if" | "then"
            | "else"
            | "elif"
            | "fi"
            | "while"
            | "do"
            | "done"
            | "until"
            | "for"
            | "case"
            | "esac"
            | "in"
            | "function"
            | "select"
            | "time"
            | "coproc"
    )
}

fn is_assignment(word: &str) -> bool {
    let Some(pos) = word.find('=') else {
        return false;
    };
    let var_name = word[..pos].strip_suffix('+').unwrap_or(&word[..pos]);
    !var_name.is_empty()
        && var_name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && var_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn has_unquoted_assignment_equal(raw: &str) -> bool {
    let mut chars = raw.chars();
    let mut in_single = false;
    let mut in_double = false;
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                chars.next();
            }
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '=' if !in_single && !in_double => return true,
            _ => {}
        }
    }
    false
}

fn is_brace_expansion(word: &str) -> bool {
    word.starts_with('{')
        && word.ends_with('}')
        && word.len() >= 3
        && (word[1..word.len() - 1].contains("..") || word.contains(','))
}

/// Tokenize a string into tokens
pub fn tokenize(input: &str) -> Vec<Token> {
    if input.trim().is_empty() {
        return Vec::new();
    }

    let mut tokens = tokenize_with_heredocs(input);
    if tokens
        .last()
        .is_some_and(|token| token.kind == TokenKind::Semicolon)
    {
        tokens.pop();
    }
    tokens
}

fn tokenize_with_heredocs(input: &str) -> Vec<Token> {
    // TODO(parse.y/redir.c): Bash parses here-documents after reading the
    // complete command and performs delimiter-specific expansion rules. This
    // line-oriented collector handles the simple `<<word` and `<<'word'`
    // forms used by early upstream alias tests.
    let mut output = Vec::new();
    let mut lines = input.lines();
    let mut position = 0;
    let mut line_number = 1;
    let mut logical_start_line = 1;
    let mut logical_line = String::new();

    while let Some(line) = lines.next() {
        if logical_line.is_empty() {
            logical_start_line = line_number;
        }
        if !logical_line.is_empty() {
            logical_line.push('\n');
        }
        logical_line.push_str(line);
        position += line.len() + 1;
        line_number += 1;

        if has_unclosed_quotes(&logical_line)
            && (is_multiline_alias_definition(&logical_line)
                || is_multiline_command_string(&logical_line)
                || is_plain_multiline_quoted_command(&logical_line))
        {
            continue;
        }

        let mut line_tokens = tokenize_plain(&logical_line);
        for token in &mut line_tokens {
            token.position = logical_start_line;
        }
        let delimiter = heredoc_delimiter(&line_tokens);
        output.append(&mut line_tokens);
        logical_line.clear();

        if let Some(delimiter) = delimiter {
            let mut body = String::new();
            for body_line in lines.by_ref() {
                position += body_line.len() + 1;
                line_number += 1;
                if body_line == delimiter {
                    break;
                }
                body.push_str(body_line);
                body.push('\n');
            }
            output.push(Token::new(TokenKind::HereDocBody, &body, position));
        }
        output.push(Token::new(TokenKind::Semicolon, ";", logical_start_line));
    }

    if !logical_line.is_empty() {
        let mut line_tokens = tokenize_plain(&logical_line);
        for token in &mut line_tokens {
            token.position = logical_start_line;
        }
        output.append(&mut line_tokens);
        output.push(Token::new(TokenKind::Semicolon, ";", logical_start_line));
    }

    output
}

fn tokenize_plain(input: &str) -> Vec<Token> {
    let lexer = Lexer::new(input);
    let mut tokens = Vec::new();
    for token in lexer {
        if token.kind == TokenKind::Eof {
            break;
        }
        tokens.push(token);
    }
    tokens
}

fn heredoc_delimiter(tokens: &[Token]) -> Option<String> {
    tokens
        .windows(2)
        .find(|pair| pair[0].kind == TokenKind::HereDoc)
        .map(|pair| pair[1].value.clone())
}

fn has_unclosed_quotes(input: &str) -> bool {
    // TODO(parse.y): Bash reads parser input with full quoting state,
    // continuations, command substitutions, arithmetic contexts, and here-doc
    // deferral. This tracks only enough single/double quote state to keep a
    // multi-line alias definition as one parser unit.
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    let mut comment_start = true;
    let mut in_comment = false;

    for ch in input.chars() {
        if in_comment {
            if ch == '\n' {
                in_comment = false;
                comment_start = true;
            }
            continue;
        }

        if escaped {
            escaped = false;
            comment_start = false;
            continue;
        }

        if ch == '\n' && !single && !double {
            comment_start = true;
            continue;
        }

        if ch == '#' && !single && !double && comment_start {
            in_comment = true;
            continue;
        }

        if ch.is_whitespace() && !single && !double {
            comment_start = true;
            continue;
        }

        if ch == '\\' && !single {
            escaped = true;
            comment_start = false;
            continue;
        }

        match ch {
            '\'' if !double => {
                single = !single;
                comment_start = false;
            }
            '"' if !single => {
                double = !double;
                comment_start = false;
            }
            _ => {
                if !single && !double {
                    comment_start = false;
                }
            }
        }
    }

    single || double
}

fn is_multiline_alias_definition(input: &str) -> bool {
    let trimmed = input.trim_start();
    trimmed.starts_with("alias ") && trimmed.contains('=')
}

fn is_multiline_command_string(input: &str) -> bool {
    let trimmed = input.trim_start();
    trimmed.contains(" -c '") || trimmed.contains(" -c \"") || trimmed.starts_with("-c '")
}

fn is_plain_multiline_quoted_command(input: &str) -> bool {
    let trimmed = input.trim_start();
    matches!(
        trimmed.split_whitespace().next(),
        Some("echo" | "printf" | "recho" | ":")
    )
}

pub struct Lexer<'a> {
    input: &'a [u8],
    position: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        Self {
            input: input.as_bytes(),
            position: 0,
        }
    }

    #[inline]
    fn at_end(&self) -> bool {
        self.position >= self.input.len()
    }

    #[inline]
    fn peek(&self) -> Option<char> {
        if self.at_end() {
            None
        } else {
            from_utf8(&self.input[self.position..]).ok()?.chars().next()
        }
    }

    #[inline]
    fn advance(&mut self) -> Option<char> {
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

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c == ' ' || c == '\t' {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn slice(&self, start: usize) -> &str {
        let end = self.position.min(self.input.len());
        from_utf8(&self.input[start..end]).unwrap_or("")
    }

    fn next_token(&mut self) -> Option<Token> {
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
                } else {
                    Some(Token::new(TokenKind::Pipe, "|", start))
                }
            }
            '&' => {
                if self.peek() == Some('&') {
                    self.advance();
                    Some(Token::new(TokenKind::And, "&&", start))
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
                    } else {
                        Some(Token::new(TokenKind::HereDoc, "<<", start))
                    }
                }
                Some('>') => {
                    self.advance();
                    Some(Token::new(TokenKind::RedirectOut, "<>", start))
                }
                _ => Some(Token::new(TokenKind::RedirectIn, "<", start)),
            },
            '>' => {
                if self.peek() == Some('>') {
                    self.advance();
                    Some(Token::new(TokenKind::Append, ">>", start))
                } else if self.peek() == Some('|') {
                    self.advance();
                    Some(Token::new(TokenKind::RedirectOut, ">|", start))
                } else {
                    Some(Token::new(TokenKind::RedirectOut, ">", start))
                }
            }
            '2' => {
                if self.peek() == Some('>') {
                    self.advance();
                    if self.peek() == Some('>') {
                        self.advance();
                        Some(Token::new(TokenKind::RedirectErrAppend, "2>>", start))
                    } else if self.peek() == Some('|') {
                        self.advance();
                        Some(Token::new(TokenKind::RedirectErr, "2>|", start))
                    } else {
                        Some(Token::new(TokenKind::RedirectErr, "2>", start))
                    }
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
                Some('(') => {
                    self.advance();
                    self.skip_cmd_subst();
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

    fn finish_word_token(&mut self, start: usize, allow_keyword: bool) -> Token {
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

    fn skip_word(&mut self) {
        while let Some(c) = self.peek() {
            if " \t\n|&;<>(){}".contains(c) {
                break;
            }
            match c {
                '`' => {
                    // TODO(parse.y/subst.c): Command substitution is part of
                    // the surrounding word. Keeping it atomic is required for
                    // assignment words such as v=`echo x`.
                    self.advance();
                    self.skip_backtick();
                }
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
                        _ => {}
                    }
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
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                '\'' => self.skip_single(),
                '"' => self.skip_double(),
                _ => {}
            }
        }
    }

    fn skip_backtick(&mut self) {
        while let Some(c) = self.advance() {
            if c == '`' {
                break;
            } else if c == '\\' {
                self.advance();
            }
        }
    }
    fn skip_single(&mut self) {
        while let Some(c) = self.advance() {
            if c == '\'' {
                break;
            }
        }
    }
    fn skip_double(&mut self) {
        while let Some(c) = self.advance() {
            if c == '"' {
                break;
            } else if c == '$' {
                match self.peek() {
                    Some('{') => {
                        self.advance();
                        self.skip_braced();
                    }
                    Some('(') => {
                        self.advance();
                        self.skip_cmd_subst();
                    }
                    _ => {}
                }
            } else if c == '\\' {
                self.advance();
            }
        }
    }
    fn skip_braced(&mut self) {
        while let Some(c) = self.advance() {
            if c == '}' {
                break;
            }
        }
    }
    fn skip_brace(&mut self) {
        let mut depth = 1usize;
        while let Some(c) = self.advance() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                '$' => match self.peek() {
                    Some('{') => {
                        self.advance();
                        self.skip_braced();
                    }
                    Some('(') => {
                        self.advance();
                        self.skip_cmd_subst();
                    }
                    _ => {}
                },
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

fn is_word_delimiter(ch: char) -> bool {
    " \t\n|&;<>(){}".contains(ch)
}

fn assignment_value_is_quoted(raw: &str) -> bool {
    let Some((_, value)) = raw.split_once('=') else {
        return false;
    };

    let mut in_backtick = false;
    let mut escaped = false;
    for ch in value.chars() {
        if escaped {
            escaped = false;
            continue;
        }

        if ch == '\\' {
            escaped = true;
            continue;
        }

        if ch == '`' {
            in_backtick = !in_backtick;
            continue;
        }

        if !in_backtick && matches!(ch, '"' | '\'') {
            return true;
        }
    }

    false
}

fn mark_quoted_assignment_value(value: &str) -> String {
    let Some((name, rhs)) = value.split_once('=') else {
        return value.to_string();
    };

    format!("{name}=\x1c{rhs}")
}

fn quoted_literal_tilde(raw: &str, value: &str) -> bool {
    value == "~"
        && ((raw.starts_with('\'') && raw.ends_with('\''))
            || (raw.starts_with('"') && raw.ends_with('"')))
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Token;
    fn next(&mut self) -> Option<Self::Item> {
        self.next_token()
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn test_tokenize_simple() {
        let tokens = tokenize("ls -la");
        assert!(tokens.len() >= 2);
        assert_eq!(tokens[0].value, "ls");
        assert_eq!(tokens[1].value, "-la");
    }

    #[test]
    fn test_tokenize_empty() {
        assert!(tokenize("").is_empty());
    }

    #[test]
    fn test_comment_skip() {
        let tokens = tokenize("ls # comment");
        assert_eq!(tokens[0].value, "ls");
        assert!(tokens
            .iter()
            .skip(1)
            .all(|token| token.kind == TokenKind::Semicolon));
    }
}

fn remove_shell_quotes(raw: &str) -> String {
    let mut out = String::new();
    let mut chars = raw.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '$' if chars.peek() == Some(&'\'') => {
                chars.next();
                let mut quoted = String::new();
                for quoted_ch in chars.by_ref() {
                    if quoted_ch == '\'' {
                        break;
                    }
                    quoted.push(quoted_ch);
                }
                out.push_str(&decode_ansi_c_quoted(&quoted));
            }
            '\'' => {
                for quoted in chars.by_ref() {
                    if quoted == '\'' {
                        break;
                    }
                    if quoted == '$' {
                        out.push('\x1f');
                    } else {
                        out.push(quoted);
                    }
                }
            }
            '"' => {
                while let Some(quoted) = chars.next() {
                    match quoted {
                        '"' => break,
                        '\\' => {
                            if let Some(escaped @ ('\\' | '"' | '$' | '`' | '\n')) =
                                chars.peek().copied()
                            {
                                chars.next();
                                if escaped != '\n' {
                                    if escaped == '$' {
                                        out.push('\x1f');
                                    } else {
                                        out.push(escaped);
                                    }
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
                    if escaped == '$' {
                        out.push('\x1f');
                    } else {
                        out.push(escaped);
                    }
                }
            }
            _ => out.push(ch),
        }
    }

    out
}

fn remove_shell_quotes_outside_backticks(raw: &str) -> String {
    let mut out = String::new();
    let mut chars = raw.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '`' => {
                out.push(ch);
                while let Some(inner) = chars.next() {
                    out.push(inner);
                    if inner == '`' {
                        break;
                    }
                    if inner == '\\' {
                        if let Some(escaped) = chars.next() {
                            out.push(escaped);
                        }
                    }
                }
            }
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
                        '`' => {
                            out.push(quoted);
                            while let Some(inner) = chars.next() {
                                out.push(inner);
                                if inner == '`' {
                                    break;
                                }
                                if inner == '\\' {
                                    if let Some(escaped) = chars.next() {
                                        out.push(escaped);
                                    }
                                }
                            }
                        }
                        '\\' => {
                            if let Some(escaped @ ('\\' | '"' | '$' | '`' | '\n')) =
                                chars.peek().copied()
                            {
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

fn decode_ansi_c_quoted(value: &str) -> String {
    // TODO(parse.y/subst.c): Bash $'...' performs full ANSI-C escape decoding,
    // including octal/hex/unicode escapes and locale-aware behavior. This
    // covers the escapes currently exercised by upstream alias tests.
    let mut output = String::new();
    let mut chars = value.chars();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            output.push(ch);
            continue;
        }

        match chars.next() {
            Some('a') => output.push('\x07'),
            Some('b') => output.push('\x08'),
            Some('e') | Some('E') => output.push('\x1b'),
            Some('f') => output.push('\x0c'),
            Some('n') => output.push('\n'),
            Some('r') => output.push('\r'),
            Some('t') => output.push('\t'),
            Some('v') => output.push('\x0b'),
            Some('\\') => output.push('\\'),
            Some('\'') => output.push('\''),
            Some('"') => output.push('"'),
            Some('?') => output.push('?'),
            Some(other) => {
                output.push('\\');
                output.push(other);
            }
            None => output.push('\\'),
        }
    }

    output
}
