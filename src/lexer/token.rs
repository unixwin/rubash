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
    /// rubash#3028: the PHYSICAL line on which this token's LAST byte was
    /// read. GNU's reader increments line_number for every physical line
    /// crossed while reading a token — including elided
    /// backslash-newline pairs (parse.y:2846 shell_getc, parse.y:2270
    /// yy_getc) — and read_token_word captures simplecmd_lineno from it
    /// at the END of a command-position word (parse.y:5849-5851), so a
    /// word split by a continuation stamps its finishing line, not its
    /// starting line. The feeder's position-stamping pass computes this
    /// from the token's end column over the same newline/join prefix
    /// counts as `position`; everywhere else it defaults to `position`
    /// (a token that cannot span physical lines ends where it starts).
    pub end_position: usize,
    pub column: usize,
    /// The LOGICAL line this token belongs to (the feeder's
    /// logical_start_line). GNU parse.y:2504 bumps line_number per
    /// PHYSICAL line, and diagnostics print the reader's line at the
    /// event (rubash#411), so `position`/`column` map tokens to physical
    /// lines; the DISCARD-family skips (execute_cmd.c rest-of-list
    /// abandonment) span a whole logical line — commands joined by
    /// backslash-newline continuations must stay comparable even though
    /// their physical lines differ. Defaults to `position` for
    /// standalone-produced tokens (single-physical-line inputs, where
    /// the two coincide).
    pub logical_line: usize,
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
    /// rubash#1544: the POSIX parse mode of the pass that produced this
    /// token — the `extglob_gate` discipline applied to the mode: a
    /// folded compound body is re-tokenized by the parser under the
    /// mode in effect when the compound was READ (GNU reads a whole
    /// complete_command before executing any of it, parse.y
    /// read_command), and a `set -o/+o posix` inside the body must not
    /// flip the body's own re-lex. Defaults to false (the shell's
    /// default mode) for synthetic tokens.
    pub posix_gate: bool,
    /// rubash#413: the word scan ended at end of input with a
    /// compound-assignment list still open (parse.y:7140
    /// parse_compound_assignment hit EOF looking for the closing `)`).
    pub compound_unclosed: bool,
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
    /// rubash#461: this token is one fragment of the still-open `=~` RHS
    /// regexp word of a `[[ ]]` conditional (GNU PST_REGEXP, parse.y:5169).
    /// GNU reads the whole RHS as ONE word (read_token_word absorbs `|` and
    /// `(...)` groups verbatim, parse.y:5443-5461) and read_token's
    /// `if (parser_state & PST_REGEXP) goto tokword' (parse.y:3663) means
    /// reserved-word recognition NEVER happens inside it. Rubash splits that
    /// word into several tokens (`|` operator, Keyword `case`/`if`/...), so
    /// this flag lets the parser's compound-boundary scanners treat those
    /// fragments as inert word data instead of grammar tokens.
    pub regexp_rhs_fragment: bool,
    /// niubash#482: this `)` token was read while the lexer's case-state
    /// tracker (scanner.rs record_token, the PST_CASEPAT port) had an open
    /// case PATTERN region — i.e. it terminates a `case ... in` pattern
    /// list, the one position where a `)` that does not pop a `(` is
    /// legal. GNU parse.y consumes pattern closers inside the case
    /// grammar, so a completeness reader can treat a flagged `)` as
    /// in-case syntax instead of a stray close; an UNFLAGGED `)` reaching
    /// the top level of an open group is GNU's `syntax error near
    /// unexpected token `)''.
    pub case_pattern_close: bool,
    /// rubash#1721: this word-path keyword token was read where a
    /// reserved word is acceptable — GNU CHECK_FOR_RESERVED_WORD
    /// (parse.y:3168) recognizes a reserved word only when
    /// reserved_word_acceptable(last_read_token) (parse.y:5899) holds,
    /// and under PST_CASEPAT (parse.y:3177-3186) only `esac' (not after
    /// `|') is recognized at all. The word path marks every
    /// keyword-VALUED word TokenKind::Keyword and lets consumers
    /// demote it; the reader stamps that demotion verdict here at read
    /// time (scanner.rs record_token, from the same reader state its
    /// history demotion uses), so completeness scans that fold tokens
    /// into a keyword stack can tell a structural keyword from an
    /// argument-position word with the same spelling (`echo if',
    /// `command -vkb if' inside a case clause). Dedicated-branch
    /// tokens (`(', `)', `{', `}', `!') are structural by construction
    /// and never consult this stamp. Defaults to false for synthetic
    /// tokens.
    pub reserved_position: bool,
}

impl Token {
    pub fn new(kind: TokenKind, value: &str, position: usize) -> Self {
        Self {
            kind,
            value: value.to_string(),
            raw: value.to_string(),
            position,
            end_position: position,
            column: position,
            logical_line: position,
            line_break: false,
            leading_ws: String::new(),
            extglob_split: false,
            extglob_gate: true,
            posix_gate: false,
            compound_unclosed: false,
            heredoc_end_line: None,
            regexp_rhs_fragment: false,
            case_pattern_close: false,
            reserved_position: false,
        }
    }

    /// Whether this token is a GRAMMAR keyword for completeness
    /// folding (keyword-stack scans): dedicated-branch tokens (`(',
    /// `)', `{', `}', `!', folded groups — values outside
    /// classification::is_keyword) are structural by construction;
    /// a word-path keyword is structural only when it was read in a
    /// reserved position (see `reserved_position`). Everything else
    /// with Keyword kind is an argument/pattern word that merely
    /// spells like a reserved word (rubash#1721).
    pub fn structural_keyword(&self) -> bool {
        self.kind == TokenKind::Keyword
            && (!super::classification::is_keyword(&self.value) || self.reserved_position)
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
            end_position: position,
            column: position,
            logical_line: position,
            line_break: false,
            leading_ws: String::new(),
            extglob_split: false,
            extglob_gate: true,
            posix_gate: false,
            compound_unclosed: false,
            heredoc_end_line: None,
            regexp_rhs_fragment: false,
            case_pattern_close: false,
            reserved_position: false,
        }
    }

    /// quoterm22 (perf21 followup): the finish_word_token call site already
    /// owns the dequoted value String; moving it in replaces one of the three
    /// per-word String allocations (`value.to_string()` copy) with a move.
    /// Semantically identical to `new_with_raw`.
    pub fn new_with_raw_owned(kind: TokenKind, value: String, raw: &str, position: usize) -> Self {
        Self {
            kind,
            value,
            raw: raw.to_string(),
            position,
            end_position: position,
            column: position,
            logical_line: position,
            line_break: false,
            leading_ws: String::new(),
            extglob_split: false,
            extglob_gate: true,
            posix_gate: false,
            compound_unclosed: false,
            heredoc_end_line: None,
            regexp_rhs_fragment: false,
            case_pattern_close: false,
            reserved_position: false,
        }
    }
}
