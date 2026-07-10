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
