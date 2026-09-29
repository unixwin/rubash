/// Token types for bash
use crate::executor::markers::DATA_DOLLAR;
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Word,
    Pipe,
    PipeErr,
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
    pub raw: String,
    pub position: usize,
    pub column: usize,
    /// Whether this separator came from a physical line break.
    pub line_break: bool,
    /// Whitespace immediately before this token in the tokenized source
    /// (parse.y keeps it so arithmetic commands can recover the raw text
    /// between `((` and `))` the way GNU bash's parse_matched_pair does).
    pub leading_ws: String,
    /// rubash#131: the word scan broke this token at a `(' that directly
    /// followed an extglob operator while the parse-time extglob gate was
    /// CLOSED. Binds the case-pattern reader's reassembly/error decision to
    /// the gate value of THIS token's line, not whatever the shopt state is
    /// when the parser later runs (GNU gates in read_token_word at read
    /// time, parse.y:5466).
    pub extglob_split: bool,
    /// rubash#131: for a folded `{ ... }' keyword token, the parse-time
    /// extglob gate value of the pass that produced it. The folding parser
    /// re-tokenizes the body later, possibly after a later line's shopt
    /// flipped the global gate; GNU decides at read time (parse.y:5466), so
    /// the body re-parse must use this snapshot, not the current value.
    /// Defaults to open so legacy producers keep accepting.
    pub extglob_gate: bool,
    /// rubash#305: for a HereDocBody token, the PHYSICAL line on which its
    /// gathering ended (the closing-delimiter line). GNU reads the '\n'
    /// that ends the delimiter line only after gather_here_documents
    /// (parse.y:3651) consumed the body lines, each advancing line_number
    /// (make_cmd.c:580), so a syntax error on that NEWLINE is REPORTED at
    /// the post-gathering line while print_offending_line (parse.y:6814)
    /// still echoes the header line. `position` carries the gather START
    /// line (the "here-document at line N" warning line), so the end line
    /// must travel separately. None for every other kind.
    pub heredoc_end_line: Option<usize>,
}

impl Token {
    pub fn new(kind: TokenKind, value: &str, position: usize) -> Self {
        Self {
            kind,
            value: value.to_string(),
            raw: value.to_string(),
            position,
            column: position,
            line_break: false,
            leading_ws: String::new(),
            extglob_split: false,
            extglob_gate: true,
            heredoc_end_line: None,
        }
    }

    /// Whether this HereDocBody token never reached its delimiter. The
    /// lexer marks unfinished bodies with a leading \x1f (behind the
    /// quoted-heredoc marker when the delimiter was quoted). Hosts use
    /// this for REPL input-completeness checks so they never have to
    /// sniff the internal marker bytes themselves.
    pub fn is_unterminated_heredoc_body(&self) -> bool {
        self.kind == TokenKind::HereDocBody
            && self
                .value
                .strip_prefix(crate::lexer::QUOTED_HEREDOC_MARKER)
                .unwrap_or(&self.value)
                .starts_with(DATA_DOLLAR)
    }

    pub fn new_with_raw(kind: TokenKind, value: &str, raw: &str, position: usize) -> Self {
        Self {
            kind,
            value: value.to_string(),
            raw: raw.to_string(),
            position,
            column: position,
            line_break: false,
            leading_ws: String::new(),
            extglob_split: false,
            extglob_gate: true,
            heredoc_end_line: None,
        }
    }
}
