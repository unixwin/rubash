//! Tier-B syntax views: the owned, span-shaped surface of the
//! Session API's syntax layer (API design Q4, decided 2026-10-25:
//! owned view types, never the internal `Ast`).
//!
//! This module's first shape is deliberately the smallest one a real
//! consumer needs. That consumer is the debugger's breakpoint
//! resolution (debugger design section 5.1, WP-8): to answer
//! "verified or not" for a `(file, line)` breakpoint it needs, per
//! parsed file, the set of lines that carry a command, the subset
//! the DEBUG-trap seam can actually stop on (the section 4.1 seam
//! distribution), and where the file's functions are defined. The
//! view is built from the internal `Ast` at the two places the
//! engine parses a whole reader unit out of a file — the `source`
//! builtin's group runner and the script driver's group loop — and
//! nowhere else: synthetic units (eval strings, alias re-parses,
//! breakpoint-condition evaluations) never register a view, so a
//! view always describes a real file the debuggee is executing.
//!
//! File identity at this layer is the exact string the engine
//! itself puts on the `BASH_SOURCE` stack for the unit: the view
//! records what the engine reported, verbatim. The session
//! core's registration point (PLAN row 11) converges that
//! spelling — and every breakpoint spelling — onto one
//! canonical identity (`debugger::resolution::file_identity`),
//! so a view registered under one spelling serves entries set
//! under any other spelling of the same file.
//!
//! Line classification (the dead-line judgment, WP-8):
//!
//! - [`LineClass::Stoppable`] — a command the seam fires before
//!   starts on this line, in a context attached to this session.
//! - [`LineClass::CommandOnly`] — a command node starts on this
//!   line, but the seam never fires *at* it: a compound head whose
//!   own line carries no simple command (`if`/`while` with the
//!   condition on a following line), or a command inside a detached
//!   context (background body, process substitution, coproc —
//!   section 6.3: those run where this session's seams do not
//!   reach). The verdict text is the design's "DEBUG trap never
//!   fires on this line".
//! - [`LineClass::NoCommand`] — no command node starts on this
//!   line: blank lines, comment lines, continuation lines, heredoc
//!   body lines, grouping openers and closing keywords on their
//!   own line (`(`, `{`, `do`, `then`, `done`, `}`, `)`). The
//!   verdict text is the design's "no command on this line".
//!
//! Command and process *substitutions* are walked in the context
//! they appear in: their bodies execute in a child executor linked
//! to this session's state (design section 6.3), so their lines are
//! seam lines when the surrounding context is attached. Subshell
//! bodies likewise keep the surrounding context (section 4.1: the
//! trap fires per simple command inside them); only the detached
//! contexts above flip the walk into detached mode.
//!
//! The module's second shape is the token view (GAP-9): the lexer
//! projected onto a caller-fed text. Unlike the views above it is
//! not built from session state — the input is the caller's text,
//! tokenized on demand in the session's posix mode — because its
//! consumers (line-editor highlighting and completion) work on the
//! line being edited, which the engine has not read. See
//! [`TokenView`].

use std::collections::BTreeSet;

use crate::{Ast, CommandNode};

/// One function definition found by the walk: everything pending
/// function-breakpoint resolution needs to prove a name is defined
/// in this file, and where its body opens (the body-open line is
/// the definition identity the engine itself records for the
/// function-entry seam).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionSite {
    pub name: String,
    /// Line of the definition command itself (`name() ...`).
    pub definition_line: u32,
    /// Line of the body's opening `{` when the parser recorded it.
    pub body_open_line: Option<u32>,
}

/// The verdict half of the dead-line judgment for one line of one
/// file (debugger design section 5.1). Resolution maps these to
/// `verified: false` plus the design's two reason strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineClass {
    /// A breakpoint here can stop the session.
    Stoppable,
    /// A command starts here but the seam never fires at it.
    CommandOnly,
    /// No command starts here.
    NoCommand,
}

/// The syntax view of one parsed file: line sets plus function
/// sites, accumulated across every reader unit of the file the
/// engine has parsed so far.
///
/// Files are parsed incrementally, one reader unit at a time, so a
/// view is *partial* while its file is still being read: it knows
/// the units it has seen (their line spans are in `spans`) and
/// nothing sound about lines beyond them. `complete` records that
/// the file's reader reached the end — only then is "no command
/// on this line" a sound verdict for a line no unit covered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyntaxView {
    /// The file identity, exactly as the engine's source stack
    /// reports it.
    pub file: String,
    /// Every line a command node starts on.
    pub command_lines: BTreeSet<u32>,
    /// The subset of `command_lines` the seam can stop on.
    pub stoppable_lines: BTreeSet<u32>,
    /// Function definitions, in first-seen order, deduplicated by
    /// (name, definition line).
    pub functions: Vec<FunctionSite>,
    /// The (first, last) command-line span of each parsed unit:
    /// the line ranges this view's knowledge actually covers.
    pub spans: Vec<(u32, u32)>,
    /// The file's reader reached its end: every line has been
    /// attributed to some unit or to none.
    pub complete: bool,
}

impl SyntaxView {
    /// Build the view of one parsed reader unit of `file`.
    pub fn from_ast(file: &str, ast: &Ast) -> SyntaxView {
        let mut view = SyntaxView {
            file: file.to_string(),
            ..SyntaxView::default()
        };
        {
            let mut walker = Walker { view: &mut view };
            walker.walk_list(&ast.commands, false);
        }
        if let (Some(first), Some(last)) = (
            view.command_lines.iter().next(),
            view.command_lines.iter().next_back(),
        ) {
            view.spans.push((*first, *last));
        }
        view
    }

    /// Fold another unit's view of the same file into this one:
    /// line sets union, function sites append when new, spans
    /// accumulate, completeness is sticky.
    pub fn merge(&mut self, other: SyntaxView) {
        debug_assert_eq!(self.file, other.file);
        self.command_lines.extend(other.command_lines);
        self.stoppable_lines.extend(other.stoppable_lines);
        for site in other.functions {
            if !self
                .functions
                .iter()
                .any(|s| s.name == site.name && s.definition_line == site.definition_line)
            {
                self.functions.push(site);
            }
        }
        self.spans.extend(other.spans);
        self.complete = self.complete || other.complete;
    }

    /// Does this view's knowledge cover `line` — does it fall
    /// inside some parsed unit's span?
    pub fn covers_line(&self, line: u32) -> bool {
        self.spans
            .iter()
            .any(|(first, last)| *first <= line && line <= *last)
    }

    /// Record that the file's reader reached its end.
    pub fn mark_complete(&mut self) {
        self.complete = true;
    }

    /// The dead-line judgment for one line of this file.
    pub fn classify_line(&self, line: u32) -> LineClass {
        if self.stoppable_lines.contains(&line) {
            LineClass::Stoppable
        } else if self.command_lines.contains(&line) {
            LineClass::CommandOnly
        } else {
            LineClass::NoCommand
        }
    }

    /// The function site proving `name` is defined in this file,
    /// when the walk has seen its definition.
    pub fn function_site(&self, name: &str) -> Option<&FunctionSite> {
        self.functions.iter().find(|site| site.name == name)
    }
}

// ----------------------------------------------------------------
// Token view (GAP-9)
// ----------------------------------------------------------------

use crate::lexer::{tokenize_with_initial_posix, Token, TokenKind};

/// The kind of one [`TokenSpan`], projected from the engine
/// lexer's token kinds onto public names. `Comment` has no lexer
/// counterpart: the lexer drops comments, so the projection
/// synthesizes comment spans from the text between tokens (see
/// [`TokenView`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKindView {
    /// An ordinary word.
    Word,
    /// A word carrying a reserved-word spelling, in whatever
    /// position — structural or demoted to a plain word by the
    /// grammar. [`TokenSpan::role`] carries the distinction.
    Keyword,
    /// An `name=value`-shaped word. The lexer stamps this kind in
    /// argument position too, so the kind alone never proves a
    /// command prefix — the role does.
    Assignment,
    /// A `$name` / `${name}` / positional / special parameter.
    Variable,
    /// A `$( )` / `` ` ` `` / `$(( ))` substitution. The lexer
    /// folds the whole substitution into one token; its interior
    /// is not projected.
    CommandSubst,
    /// A brace-expansion word.
    BraceExpand,
    /// `|`
    Pipe,
    /// `|&`
    PipeErr,
    /// `;` — including the line-break separator the lexer reports
    /// for a physical newline.
    Semicolon,
    /// `&`
    Background,
    /// `&&`
    And,
    /// `||`
    Or,
    /// `>`
    RedirectOut,
    /// `<`
    RedirectIn,
    /// `>>`
    Append,
    /// The `2>` family; an fd prefix rides inside the span text.
    RedirectErr,
    /// The `2>>` family.
    RedirectErrAppend,
    /// The `<<` / `<<-` operator.
    HereDoc,
    /// The `<<<` operator.
    HereString,
    /// A here-doc body: the lines between the operator line and
    /// the delimiter line. The span covers the body lines as fed
    /// (for `<<-`, leading tabs included — the lexer's raw body
    /// has them stripped; the input slice is the truth).
    HereDocBody,
    /// A `#` comment, synthesized from inter-token text.
    Comment,
}

impl TokenKindView {
    /// Whether spans of this kind can be the word under a cursor:
    /// the word-shaped kinds (words, keywords, assignments,
    /// variables, substitutions, brace expansions).
    pub fn is_word_shaped(self) -> bool {
        matches!(
            self,
            TokenKindView::Word
                | TokenKindView::Keyword
                | TokenKindView::Assignment
                | TokenKindView::Variable
                | TokenKindView::CommandSubst
                | TokenKindView::BraceExpand
        )
    }

    /// Whether spans of this kind separate one command segment
    /// from the next (`;`, newline, `&`, `&&`, `||`, `|`, `|&`).
    pub fn is_separator(self) -> bool {
        matches!(
            self,
            TokenKindView::Semicolon
                | TokenKindView::Background
                | TokenKindView::And
                | TokenKindView::Or
                | TokenKindView::Pipe
                | TokenKindView::PipeErr
        )
    }
}

/// What a span is doing in its command, folded at projection time
/// from the lexer's own stamps (the reserved-word table it stamps
/// kinds and positions with, the case-pattern close stamp) and
/// the engine's builtin table
/// (`Executor::builtin_command_names`). Only word-shaped spans
/// carry a non-`Plain` role; operators, separators, here-doc
/// bodies and comments are always `Plain`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenRole {
    /// A structural reserved word in the position the grammar
    /// gives it (`if`, `then`, `do`, `{`, a case `in`, …).
    Reserved,
    /// A word in command position naming an engine builtin.
    Builtin,
    /// An assignment word in a command's prefix.
    Assignment,
    /// The word a redirection operator takes as its target.
    RedirectTarget,
    /// Everything else: command names that are not builtins,
    /// arguments, case patterns and their subjects, function
    /// names, `for` variables and their word lists, here-doc
    /// delimiters, and every non-word span.
    Plain,
}

/// One projected token over the input text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenSpan {
    /// Absolute byte offset of the span's first byte in the input
    /// as fed.
    pub start: usize,
    /// Absolute byte offset one past the span's last byte.
    pub end: usize,
    /// The span's projected kind.
    pub kind: TokenKindView,
    /// The span's raw source slice, verbatim from the input.
    pub text: String,
    /// The span's dequoted value for word-shaped kinds: quote
    /// removal over the raw slice, the same removal the engine's
    /// completion machinery performs (`dequote_word` below).
    /// Derived from the raw slice alone — the lexer's token
    /// values carry in-band carrier bytes for quoting state and
    /// never cross this surface. For non-word kinds, the raw text
    /// itself.
    pub value: String,
    /// Whether the span stands where a command word may stand:
    /// the first word of a command segment, its assignment
    /// prefixes, and the structural keywords that occupy that
    /// position. Always false for non-word spans.
    pub command_position: bool,
    /// The span's folded role.
    pub role: TokenRole,
    /// The lexer's case-pattern close stamp, verbatim: true for
    /// the `)` that closes a `case` pattern list. Consumers
    /// re-deriving case structure from the flat span stream need
    /// exactly this bit; it is carried, not folded, because the
    /// fold already consumed it.
    pub case_pattern_close: bool,
}

/// The lexer projected onto a caller-fed text: a flat, ordered
/// span stream covering the text's tokens, with the derivations
/// line-editor consumers (highlighting, completion) otherwise
/// each re-implement with a private scanner.
///
/// Recorded difference from this module's other views (GAP-3/4):
/// those capture session *state*; this one's input is caller
/// *text*, tokenized on demand in the session's posix mode
/// (`Session::token_view`). Spans carry absolute byte offsets
/// over the input as fed.
///
/// What the projection adds over the raw token stream:
///
/// - **Comment spans.** The lexer drops comments entirely; a
///   span is synthesized for each `#` comment found in the text
///   between tokens (a `#` at a word-start position in a gap —
///   gap text holds only whitespace, continuations and comments,
///   so every `#` in a gap qualifies).
/// - **Command position and roles**, folded from the tokens'
///   stamps and engine tables (see [`TokenRole`]). Nothing in
///   `src/lexer/` changes for this view: every fact is derived
///   at projection time.
/// - **Dequoted values** built from the raw source slices (see
///   [`TokenSpan::value`]).
///
/// Two lexer shapes set the span arithmetic. A line-break
/// separator's position stamps are meaningless (the lexer reports
/// line 1, column 1): its span is the newline byte it stands for,
/// found by anchoring forward from the previous span. A here-doc
/// body's stamps likewise do not address the input (its raw text
/// is post-processed — markers prefixed, `<<-` tabs stripped):
/// its span is computed from the body's line extent, which the
/// lexer does record (gather-start line, delimiter line).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenView {
    /// The spans, in source order.
    pub tokens: Vec<TokenSpan>,
}

impl TokenView {
    /// Project `text` through the engine lexer. `posix` is the
    /// session's posix mode: tokenization derives in it, exactly
    /// as `Session::parse` derives (one recorded delta: in posix
    /// mode `time` is not a reserved word).
    pub(crate) fn project(text: &str, posix: bool) -> TokenView {
        let tokens = tokenize_with_initial_posix(text, posix);
        let line_starts = line_start_offsets(text);
        let mut spans: Vec<TokenSpan> = Vec::new();
        let mut fold = Fold::default();
        let mut placed_end = 0usize;
        let mut heredoc_cursor = 0usize;
        for token in &tokens {
            if token.kind == TokenKind::Eof {
                continue;
            }
            let (start, end) =
                place_token(text, token, &line_starts, placed_end, &mut heredoc_cursor);
            push_comment_spans(text, placed_end, start, &mut spans);
            let kind = token_kind_view(&token.kind);
            let raw_text = &text[start..end];
            let value = span_value(kind, raw_text);
            let (command_position, role) = fold.apply(token, kind, &value);
            spans.push(TokenSpan {
                start,
                end,
                kind,
                text: raw_text.to_string(),
                value,
                command_position,
                role,
                case_pattern_close: token.case_pattern_close,
            });
            placed_end = end;
        }
        push_comment_spans(text, placed_end, text.len(), &mut spans);
        TokenView { tokens: spans }
    }

    /// The word-shaped span the cursor is completing: the span
    /// with `start < cursor <= end`. `None` at a span start, on
    /// whitespace, on an operator, and everywhere else no word is
    /// under the cursor — the same verdict the host's segment scan
    /// reaches for a segment ending in unescaped whitespace.
    pub fn word_at(&self, cursor: usize) -> Option<&TokenSpan> {
        self.tokens
            .iter()
            .find(|span| span.kind.is_word_shaped() && span.start < cursor && cursor <= span.end)
    }

    /// The word-shaped spans of the cursor's command segment that
    /// start before the cursor, in source order: the segment is
    /// the text after the last separator before the cursor. A
    /// span straddling the cursor counts (it starts before it) —
    /// a completer trimming the in-progress word off the end of
    /// this list gets the words strictly before it.
    pub fn segment_words_before(&self, cursor: usize) -> Vec<&TokenSpan> {
        let segment_start = self
            .last_separator_before(cursor)
            .map(|span| span.end)
            .unwrap_or(0);
        self.tokens
            .iter()
            .filter(|span| {
                span.kind.is_word_shaped() && span.start >= segment_start && span.start < cursor
            })
            .collect()
    }

    /// The last separator span ending at or before the cursor.
    pub fn last_separator_before(&self, cursor: usize) -> Option<&TokenSpan> {
        self.tokens
            .iter()
            .rev()
            .find(|span| span.kind.is_separator() && span.end <= cursor)
    }
}

/// Map one lexer kind onto its public projection. `Eof` never
/// reaches here (the projection skips it); it maps to `Word` only
/// to keep the match exhaustive.
fn token_kind_view(kind: &TokenKind) -> TokenKindView {
    match kind {
        TokenKind::Word => TokenKindView::Word,
        TokenKind::Keyword => TokenKindView::Keyword,
        TokenKind::Assignment => TokenKindView::Assignment,
        TokenKind::Variable => TokenKindView::Variable,
        TokenKind::CommandSubst => TokenKindView::CommandSubst,
        TokenKind::BraceExpand => TokenKindView::BraceExpand,
        TokenKind::Pipe => TokenKindView::Pipe,
        TokenKind::PipeErr => TokenKindView::PipeErr,
        TokenKind::Semicolon => TokenKindView::Semicolon,
        TokenKind::Background => TokenKindView::Background,
        TokenKind::And => TokenKindView::And,
        TokenKind::Or => TokenKindView::Or,
        TokenKind::RedirectOut => TokenKindView::RedirectOut,
        TokenKind::RedirectIn => TokenKindView::RedirectIn,
        TokenKind::Append => TokenKindView::Append,
        TokenKind::RedirectErr => TokenKindView::RedirectErr,
        TokenKind::RedirectErrAppend => TokenKindView::RedirectErrAppend,
        TokenKind::HereDoc => TokenKindView::HereDoc,
        TokenKind::HereString => TokenKindView::HereString,
        TokenKind::HereDocBody => TokenKindView::HereDocBody,
        TokenKind::Eof => TokenKindView::Word,
    }
}

/// Byte offset of the start of 1-based `line` in `text`, from the
/// precomputed line-start table; the end of the text when the
/// line is past the last one.
fn line_start(line_starts: &[usize], line: usize, text_len: usize) -> usize {
    line.checked_sub(1)
        .and_then(|index| line_starts.get(index).copied())
        .unwrap_or(text_len)
}

/// The byte offset of every line start in `text`.
fn line_start_offsets(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(index + 1);
        }
    }
    starts
}

/// Place one token over the input: the `(start, end)` byte span
/// its raw text occupies, in stream order (never before
/// `placed_end`). See [`TokenView`] for the two stamp-defying
/// shapes (line-break separators, here-doc bodies).
fn place_token(
    text: &str,
    token: &Token,
    line_starts: &[usize],
    placed_end: usize,
    heredoc_cursor: &mut usize,
) -> (usize, usize) {
    if token.kind == TokenKind::HereDocBody {
        // The body's first line follows the line the gather
        // started on; a later body on the same operator line
        // starts after the previous body's delimiter line.
        let after_operator_line = line_start(line_starts, token.position + 1, text.len());
        let start = after_operator_line.max(*heredoc_cursor);
        let end = if token.is_unterminated_heredoc_body() {
            text.len()
        } else if let Some(delimiter_line) = token.heredoc_end_line {
            line_start(line_starts, delimiter_line, text.len())
        } else {
            text.len()
        };
        *heredoc_cursor = match token.heredoc_end_line {
            Some(delimiter_line) => line_start(line_starts, delimiter_line + 1, text.len()),
            None => text.len(),
        };
        return (start, end.max(start));
    }
    if token.line_break && token.kind == TokenKind::Semicolon {
        // The line-break separator: its stamps are meaningless
        // (line 1, column 1); it stands for the next newline byte
        // at or after the previous span.
        return match text[placed_end..].find('\n') {
            Some(offset) => (placed_end + offset, placed_end + offset + 1),
            None => (placed_end, placed_end),
        };
    }
    // The common case: the column is a byte offset within the
    // token's logical line, whose start the table gives.
    let candidate = line_start(line_starts, token.logical_line, text.len()) + token.column;
    if candidate >= placed_end
        && text.get(candidate..candidate + token.raw.len()) == Some(token.raw.as_str())
    {
        return (candidate, candidate + token.raw.len());
    }
    // Anything the stamps cannot address (defensive; not observed
    // for sound token streams): anchor on the raw text itself,
    // forward from the previous span.
    if let Some(offset) = text[placed_end..].find(token.raw.as_str()) {
        let start = placed_end + offset;
        return (start, start + token.raw.len());
    }
    let start = candidate.max(placed_end).min(text.len());
    (start, (start + token.raw.len()).min(text.len()))
}

/// Synthesize comment spans for the `#` comments in the gap
/// `text[from..to]` between two placed tokens (or a text edge).
/// Gap text holds only whitespace, backslash-newline continuations
/// and comments, so a `#` at a word-start position in the gap —
/// after whitespace, a newline, a continuation, or the gap start —
/// always opens a comment, which runs to the next newline.
fn push_comment_spans(text: &str, from: usize, to: usize, spans: &mut Vec<TokenSpan>) {
    if from >= to {
        return;
    }
    let bytes = text.as_bytes();
    let mut index = from;
    let mut word_start = true;
    while index < to {
        match bytes[index] {
            b'#' if word_start => {
                let mut end = index;
                while end < to && bytes[end] != b'\n' {
                    end += 1;
                }
                let raw = &text[index..end];
                spans.push(TokenSpan {
                    start: index,
                    end,
                    kind: TokenKindView::Comment,
                    text: raw.to_string(),
                    value: raw.to_string(),
                    command_position: false,
                    role: TokenRole::Plain,
                    case_pattern_close: false,
                });
                index = end;
                word_start = false;
            }
            b'\n' | b' ' | b'\t' | b'\r' => {
                word_start = true;
                index += 1;
            }
            b'\\' => {
                // A continuation glues lines; a `#` after it keeps
                // the word position it had before it.
                index += 1;
            }
            _ => {
                word_start = false;
                index += 1;
            }
        }
    }
}

/// The span value for one span: quote removal over the raw slice
/// for word-shaped kinds, the raw text itself otherwise.
fn span_value(kind: TokenKindView, raw_text: &str) -> String {
    if kind.is_word_shaped() {
        dequote_word(raw_text)
    } else {
        raw_text.to_string()
    }
}

/// Quote removal over one word, in the shape the host's editor
/// consumers are built on: quotes group and vanish, a backslash
/// quotes the next character literally everywhere except inside
/// single quotes (inside double quotes it likewise yields the
/// next character verbatim — the host-shaped rule, which differs
/// from GNU's execution dequote only for `\` before a non-special
/// character inside double quotes, where GNU keeps the backslash).
/// This is the same state machine as the engine's completion
/// dequote (`builtins::complete::dequote_completion_word`,
/// rubash#1087, itself over GNU's completion word handling —
/// `subst.c:4807 dequote_string()` is the execution-side owner),
/// ported here because that one is private to its builtin.
/// A trailing lone backslash survives, as does an unterminated
/// quote's content.
fn dequote_word(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    let mut chars = word.chars();
    let mut quote: Option<char> = None;
    while let Some(ch) = chars.next() {
        match quote {
            Some('\'') => {
                if ch == '\'' {
                    quote = None;
                } else {
                    out.push(ch);
                }
            }
            Some('"') => {
                if ch == '"' {
                    quote = None;
                } else if ch == '\\' {
                    match chars.next() {
                        Some(next) => out.push(next),
                        None => out.push('\\'),
                    }
                } else {
                    out.push(ch);
                }
            }
            _ => match ch {
                '\'' | '"' => quote = Some(ch),
                '\\' => match chars.next() {
                    Some(next) => out.push(next),
                    None => out.push('\\'),
                },
                _ => out.push(ch),
            },
        }
    }
    out
}

/// Whether the engine's builtin table names `name`.
fn is_builtin_name(name: &str) -> bool {
    crate::executor::Executor::builtin_command_names()
        .binary_search(&name)
        .is_ok()
}

/// The folded position/role state over one projection. See
/// [`TokenRole`] for the sources; the transitions mirror the
/// grammar the lexer itself stamps (a structural keyword is one
/// whose stamp says so — the same stamp the parser consumes).
struct Fold {
    /// The next word may be a command word (segment start, after
    /// a separator or an opening keyword). True at the start of
    /// the text.
    expect_command: bool,
    /// The next word-shaped span is a redirection target.
    redirect_target: bool,
    /// The next word-shaped span is a function name (after the
    /// `function` keyword).
    function_name_next: bool,
    /// After a structural `time`: its dash-options ride before
    /// the command word.
    time_prefix: bool,
    /// The `for` / `select` variable-and-list walk.
    for_phase: ForPhase,
    /// Open `case` clauses, innermost last.
    case_stack: Vec<CaseFrame>,
}

impl Default for Fold {
    fn default() -> Self {
        Fold {
            expect_command: true,
            redirect_target: false,
            function_name_next: false,
            time_prefix: false,
            for_phase: ForPhase::None,
            case_stack: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum ForPhase {
    #[default]
    None,
    /// The loop variable is next.
    Variable,
    /// The word list (including the demoted `in`) runs to `do`
    /// or a separator.
    List,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CasePhase {
    /// The subject word, then the structural `in`.
    Head,
    /// Pattern words and `|` alternation, to the close stamp.
    Patterns,
    /// The clause body, to its `;;`-family terminator or `esac`.
    Body,
}

struct CaseFrame {
    phase: CasePhase,
    subject_seen: bool,
}

/// A `case` clause terminator as the lexer emits it: a plain word
/// (the terminators are not operators in the token stream).
fn is_case_terminator(raw: &str) -> bool {
    matches!(raw, ";;" | ";&" | ";;&")
}

impl Fold {
    /// Fold one token into the state; returns its
    /// `(command_position, role)`.
    fn apply(&mut self, token: &Token, kind: TokenKindView, value: &str) -> (bool, TokenRole) {
        // Redirection operators: the next word-shaped span is the
        // target, whatever position the command is in.
        if matches!(
            kind,
            TokenKindView::RedirectOut
                | TokenKindView::RedirectIn
                | TokenKindView::Append
                | TokenKindView::RedirectErr
                | TokenKindView::RedirectErrAppend
                | TokenKindView::HereDoc
                | TokenKindView::HereString
        ) {
            self.redirect_target = true;
            return (false, TokenRole::Plain);
        }
        // Here-doc bodies are inert data.
        if kind == TokenKindView::HereDocBody {
            return (false, TokenRole::Plain);
        }
        let word_shaped = kind.is_word_shaped();
        if self.redirect_target {
            self.redirect_target = false;
            if word_shaped {
                return (false, TokenRole::RedirectTarget);
            }
            // A non-word after a redirect operator is malformed
            // input; fall through and classify it on its own feet.
        }
        // `case` phases intercept before the general fold.
        if !self.case_stack.is_empty() {
            let top = self.case_stack.len() - 1;
            match self.case_stack[top].phase {
                CasePhase::Head => {
                    if word_shaped && !self.case_stack[top].subject_seen {
                        self.case_stack[top].subject_seen = true;
                        return (false, TokenRole::Plain);
                    }
                    if kind == TokenKindView::Keyword
                        && token.structural_keyword()
                        && token.raw == "in"
                    {
                        self.case_stack[top].phase = CasePhase::Patterns;
                        return (false, TokenRole::Reserved);
                    }
                }
                CasePhase::Patterns => {
                    if token.case_pattern_close {
                        self.case_stack[top].phase = CasePhase::Body;
                        self.expect_command = true;
                        return (false, TokenRole::Plain);
                    }
                    if kind == TokenKindView::Keyword && token.structural_keyword() {
                        if token.raw == "(" {
                            return (false, TokenRole::Reserved);
                        }
                        // A structural keyword is not a pattern:
                        // `esac` (which pops the frame) and a
                        // nested `case` fall through to the
                        // general fold below.
                    } else if kind == TokenKindView::Pipe || word_shaped {
                        return (false, TokenRole::Plain);
                    }
                }
                CasePhase::Body => {
                    if word_shaped && is_case_terminator(&token.raw) {
                        self.case_stack[top].phase = CasePhase::Patterns;
                        return (false, TokenRole::Plain);
                    }
                    // `esac` and nested constructs fall through.
                }
            }
        }
        match kind {
            TokenKindView::Semicolon
            | TokenKindView::Background
            | TokenKindView::And
            | TokenKindView::Or
            | TokenKindView::Pipe
            | TokenKindView::PipeErr => {
                self.expect_command = true;
                self.for_phase = ForPhase::None;
                self.time_prefix = false;
                self.function_name_next = false;
                (false, TokenRole::Plain)
            }
            TokenKindView::Keyword if token.structural_keyword() => {
                self.structural_keyword(&token.raw)
            }
            _ if word_shaped => self.word(value, kind, &token.raw),
            _ => (false, TokenRole::Plain),
        }
    }

    /// The structural-keyword arm: `role` is `Reserved`, command
    /// position is the position the keyword itself stands in, and
    /// the state transition is the grammar's.
    fn structural_keyword(&mut self, raw: &str) -> (bool, TokenRole) {
        let command_position = self.expect_command;
        match raw {
            "if" | "while" | "until" => {
                self.expect_command = true;
            }
            "then" | "do" | "else" | "elif" => {
                self.expect_command = true;
                self.for_phase = ForPhase::None;
            }
            "fi" | "done" => {
                self.expect_command = false;
            }
            "for" | "select" => {
                self.expect_command = false;
                self.for_phase = ForPhase::Variable;
            }
            "case" => {
                self.case_stack.push(CaseFrame {
                    phase: CasePhase::Head,
                    subject_seen: false,
                });
                self.expect_command = false;
            }
            "esac" => {
                self.case_stack.pop();
                self.expect_command = false;
            }
            "function" => {
                self.function_name_next = true;
                self.expect_command = false;
            }
            "time" => {
                self.time_prefix = true;
                self.expect_command = true;
            }
            "coproc" => {
                self.expect_command = true;
            }
            "{" if raw.len() > 1 => {
                // A folded brace group is one complete command.
                self.expect_command = false;
            }
            "(" => {
                // A subshell `(` stands at command position
                // itself (the state already expects a command, so
                // nothing changes); a `(` after a command word is
                // a function definition's parameter list and opens
                // no command position.
            }
            "{" | "!" => {
                self.expect_command = true;
            }
            "}" | ")" => {
                self.expect_command = false;
            }
            _ => {}
        }
        (command_position, TokenRole::Reserved)
    }

    /// The word arm (word-shaped kinds, demoted keywords
    /// included).
    fn word(&mut self, value: &str, kind: TokenKindView, raw: &str) -> (bool, TokenRole) {
        if self.function_name_next {
            self.function_name_next = false;
            return (false, TokenRole::Plain);
        }
        match self.for_phase {
            ForPhase::Variable => {
                self.for_phase = ForPhase::List;
                return (false, TokenRole::Plain);
            }
            ForPhase::List => return (false, TokenRole::Plain),
            ForPhase::None => {}
        }
        if self.time_prefix && raw.starts_with('-') {
            // `time`'s own options ride before the command word.
            return (false, TokenRole::Plain);
        }
        self.time_prefix = false;
        if self.expect_command {
            if kind == TokenKindView::Assignment {
                // A prefix assignment: the command word is still
                // to come, so command position stays open.
                return (true, TokenRole::Assignment);
            }
            self.expect_command = false;
            let role = if is_builtin_name(value) {
                TokenRole::Builtin
            } else {
                TokenRole::Plain
            };
            (true, role)
        } else {
            (false, TokenRole::Plain)
        }
    }
}

/// The AST walk. `detached` is the section 6.3 context flag: once a
/// subtree runs detached from the session (background body, process
/// substitution, coproc body), its lines are command lines but
/// never stoppable lines, however deeply attached shapes nest
/// inside it.
struct Walker<'a> {
    view: &'a mut SyntaxView,
}

impl Walker<'_> {
    fn record(&mut self, node: &CommandNode, stoppable: bool, detached: bool) {
        if let Some(line) = node_line(node) {
            self.view.command_lines.insert(line);
            if stoppable && !detached {
                self.view.stoppable_lines.insert(line);
            }
        }
    }

    fn walk_list(&mut self, nodes: &[CommandNode], detached: bool) {
        for node in nodes {
            self.walk_node(node, detached);
        }
    }

    fn walk_node(&mut self, node: &CommandNode, detached: bool) {
        // A `&` on the node itself launches the whole command
        // detached (section 6.3), whatever shape it has.
        let detached = detached || node.background;
        // Substitutions inside this node's own words execute as
        // part of the node, in the node's context: process
        // substitutions run in separate processes (detached).
        // (Command substitutions are not stored on the node — the
        // wt44/parse4 record vectors were retired; see nodes.rs.)
        for substitution in &node.process_substitutions {
            self.walk_list(&substitution.commands, true);
        }

        if let Some(pipeline) = &node.pipeline_command {
            self.record(node, false, detached);
            self.walk_list(&pipeline.stages, detached);
            return;
        }
        if let Some(list) = &node.and_or_list {
            self.record(node, false, detached);
            self.walk_list(&list.commands, detached);
            return;
        }
        if let Some(time) = &node.time_command {
            self.record(node, false, detached);
            self.walk_node(&time.command, detached);
            return;
        }
        if let Some(background) = &node.background_command {
            self.record(node, false, detached);
            self.walk_node(&background.command, true);
            return;
        }
        if let Some(inverted) = &node.inverted_command {
            self.record(node, false, detached);
            self.walk_node(&inverted.command, detached);
            return;
        }
        if let Some(for_command) = &node.for_command {
            // The for head carries a seam (section 4.1: once per
            // iteration, before the body) at the head's own line.
            self.record(node, true, detached);
            self.walk_list(&for_command.body, detached);
            return;
        }
        if node.arithmetic_command.is_some() {
            self.record(node, true, detached);
            return;
        }
        if let Some(if_command) = &node.if_command {
            // No seam at the `if` head itself: the seams are the
            // condition and body commands (section 4.1). A head
            // line shared with the first condition command is
            // stoppable through that command, not through the head.
            self.record(node, false, detached);
            self.walk_list(&if_command.condition, detached);
            self.walk_list(&if_command.then_body, detached);
            for branch in &if_command.elif_branches {
                self.walk_list(&branch.condition, detached);
                self.walk_list(&branch.body, detached);
            }
            if let Some(else_body) = &if_command.else_body {
                self.walk_list(else_body, detached);
            }
            return;
        }
        if let Some(loop_command) = &node.loop_command {
            // Same head rule as `if`: the condition commands carry
            // the loop's seam (once per check, section 4.1).
            self.record(node, false, detached);
            self.walk_list(&loop_command.condition, detached);
            self.walk_list(&loop_command.body, detached);
            return;
        }
        if node.conditional_command.is_some() {
            self.record(node, true, detached);
            return;
        }
        if let Some(subshell) = &node.subshell_command {
            // No line of the head's own is recorded: the parser
            // attributes a subshell node's line to its *closing*
            // line, and neither the `(` line nor the `)` line has a
            // command starting on it — the body's commands record
            // their own lines.
            self.walk_list(&subshell.body, detached);
            return;
        }
        if let Some(case) = &node.case_command {
            // The case head carries the seam that evaluates the
            // word and dispatches (section 4.1); clause bodies are
            // ordinary commands at their own lines.
            self.record(node, true, detached);
            for clause in &case.clauses {
                self.walk_list(&clause.body, detached);
            }
            return;
        }
        if let Some(select) = &node.select_command {
            // The select head is a seam like the for head (it is
            // the for family's other member, section 4.1).
            self.record(node, true, detached);
            self.walk_list(&select.body, detached);
            return;
        }
        if let Some(function) = &node.function_command {
            // The definition line is stoppable, but not because a
            // seam fires before the definition executes — none
            // does (the executor's seam admission skips definition
            // commands outright, matching GNU's dispatch). It is
            // stoppable because the function-entry seam reports
            // this line: at entry the engine stamps the current
            // line with the definition line, so a line breakpoint
            // here binds and fires at each call's entry seam.
            self.record(node, true, detached);
            if let Some(line) = node_line(node) {
                let site = FunctionSite {
                    name: function.name.clone(),
                    definition_line: line,
                    body_open_line: function.body_open_line.and_then(line_u32),
                };
                if !self
                    .view
                    .functions
                    .iter()
                    .any(|s| s.name == site.name && s.definition_line == site.definition_line)
                {
                    self.view.functions.push(site);
                }
            }
            self.walk_list(&function.body, detached);
            return;
        }
        if let Some(group) = &node.brace_group {
            // Grouping head, same rule as the subshell: a bare `{`
            // line starts no command; the body records its lines.
            self.walk_list(&group.body, detached);
            return;
        }
        if let Some(coproc) = &node.coproc_command {
            // A coproc body runs in its own process: command lines,
            // never stoppable (section 6.3's detached rule, applied
            // to the one compound shape the table leaves implicit).
            // The simple form (`coproc cmd`) is itself the command,
            // so its line counts; the compound form's node line is
            // attributed like a subshell's and is not recorded.
            if coproc.body.is_none() {
                self.record(node, false, detached);
            }
            if let Some(body) = &coproc.body {
                self.walk_list(body, true);
            }
            return;
        }
        // What remains is a simple command — words, assignments,
        // redirects only, or nothing the walk must descend into.
        // Section 4.1: the seam fires before every simple command,
        // assignments-only commands included.
        self.record(node, true, detached);
    }
}

fn line_u32(line: usize) -> Option<u32> {
    u32::try_from(line).ok().filter(|line| *line > 0)
}

fn node_line(node: &CommandNode) -> Option<u32> {
    node.line.and_then(line_u32)
}

#[cfg(test)]
mod tests {
    use super::{LineClass, SyntaxView};

    fn view_of(source: &str) -> SyntaxView {
        let tokens = crate::lexer::tokenize(source);
        let ast = crate::parser::parse(&tokens);
        SyntaxView::from_ast("fixture.sh", &ast)
    }

    #[test]
    fn simple_commands_are_stoppable_at_their_lines() {
        let view = view_of("echo one\necho two\nX=3\n");
        assert_eq!(view.classify_line(1), LineClass::Stoppable);
        assert_eq!(view.classify_line(2), LineClass::Stoppable);
        assert_eq!(
            view.classify_line(3),
            LineClass::Stoppable,
            "assignments-only commands carry a seam (design 4.1)"
        );
    }

    #[test]
    fn blank_comment_and_closing_lines_have_no_command() {
        let view = view_of("# a comment\n\necho hi\n");
        assert_eq!(view.classify_line(1), LineClass::NoCommand);
        assert_eq!(view.classify_line(2), LineClass::NoCommand);
        assert_eq!(view.classify_line(3), LineClass::Stoppable);
        assert_eq!(
            view.classify_line(99),
            LineClass::NoCommand,
            "a line past the end of the file has no command either"
        );
    }

    #[test]
    fn heredoc_body_lines_have_no_command() {
        let view = view_of("cat <<EOF\nbody one\nbody two\nEOF\necho after\n");
        assert_eq!(view.classify_line(1), LineClass::Stoppable);
        assert_eq!(view.classify_line(2), LineClass::NoCommand);
        assert_eq!(view.classify_line(3), LineClass::NoCommand);
        assert_eq!(view.classify_line(4), LineClass::NoCommand);
        assert_eq!(view.classify_line(5), LineClass::Stoppable);
    }

    #[test]
    fn continuation_lines_have_no_command_of_their_own() {
        let view = view_of("echo one \\\n    two\necho next\n");
        assert_eq!(view.classify_line(1), LineClass::Stoppable);
        assert_eq!(view.classify_line(2), LineClass::NoCommand);
        assert_eq!(view.classify_line(3), LineClass::Stoppable);
    }

    #[test]
    fn if_head_line_is_stoppable_only_through_its_condition() {
        // Condition on the head line: the line stops (through the
        // condition command's own seam).
        let view = view_of("if test -f x; then\n  echo yes\nfi\n");
        assert_eq!(view.classify_line(1), LineClass::Stoppable);
        assert_eq!(view.classify_line(2), LineClass::Stoppable);
        assert_eq!(view.classify_line(3), LineClass::NoCommand);
        // Condition on the following line: the bare `if` line is a
        // command line the trap never fires at.
        let view = view_of("if\n  test -f x\nthen\n  echo yes\nfi\n");
        assert_eq!(view.classify_line(1), LineClass::CommandOnly);
        assert_eq!(view.classify_line(2), LineClass::Stoppable);
        assert_eq!(view.classify_line(3), LineClass::NoCommand);
    }

    #[test]
    fn do_on_its_own_line_has_no_command() {
        let view = view_of("while read -r x\ndo\n  echo \"$x\"\ndone\n");
        assert_eq!(
            view.classify_line(1),
            LineClass::Stoppable,
            "the condition command shares the head line"
        );
        assert_eq!(view.classify_line(2), LineClass::NoCommand);
        assert_eq!(view.classify_line(3), LineClass::Stoppable);
        assert_eq!(view.classify_line(4), LineClass::NoCommand);
    }

    #[test]
    fn function_definition_registers_site_and_body_lines() {
        let view = view_of("f() {\n  echo in-f\n}\nf\n");
        let site = view.function_site("f").expect("definition recorded");
        assert_eq!(site.definition_line, 1);
        assert_eq!(view.classify_line(1), LineClass::Stoppable);
        assert_eq!(view.classify_line(2), LineClass::Stoppable);
        assert_eq!(
            view.classify_line(3),
            LineClass::NoCommand,
            "the closing brace line carries no command node"
        );
        assert_eq!(view.classify_line(4), LineClass::Stoppable);
        assert!(view.function_site("missing").is_none());
    }

    #[test]
    fn detached_contexts_are_command_lines_but_never_stoppable() {
        let view = view_of("sleep 1 &\necho main\n");
        assert_eq!(view.classify_line(1), LineClass::CommandOnly);
        assert_eq!(view.classify_line(2), LineClass::Stoppable);
        let view = view_of("echo >(cat > /dev/null)\necho main\n");
        assert_eq!(
            view.classify_line(1),
            LineClass::Stoppable,
            "the owning command itself is attached; only the \
             substitution body is detached"
        );
        assert_eq!(view.classify_line(2), LineClass::Stoppable);
    }

    #[test]
    fn subshell_and_comsub_bodies_keep_their_context() {
        let view = view_of("(\n  echo inner\n)\n");
        assert_eq!(
            view.classify_line(1),
            LineClass::NoCommand,
            "a bare `(` line starts no command; the body does"
        );
        assert_eq!(view.classify_line(2), LineClass::Stoppable);
        assert_eq!(view.classify_line(3), LineClass::NoCommand);
        let view = view_of("X=$(echo inner)\necho outer\n");
        assert_eq!(view.classify_line(1), LineClass::Stoppable);
        assert_eq!(view.classify_line(2), LineClass::Stoppable);
    }

    #[test]
    fn merge_unions_units_of_one_file() {
        let mut first = view_of("f() {\n  echo a\n}\n");
        let second = view_of("f\necho done\n");
        first.merge(second);
        assert!(first.function_site("f").is_some());
        assert_eq!(first.classify_line(1), LineClass::Stoppable);
        assert_eq!(first.classify_line(2), LineClass::Stoppable);
    }

    #[test]
    fn coverage_spans_track_parsed_units_and_completion_is_sticky() {
        let mut view = view_of("echo one\n# mid\necho three\n");
        assert!(view.covers_line(1));
        assert!(
            view.covers_line(2),
            "a line between two commands of one unit is covered"
        );
        assert!(view.covers_line(3));
        assert!(
            !view.covers_line(9),
            "a line past every parsed unit is not covered"
        );
        assert!(!view.complete);
        let other = view_of("echo late\n");
        view.merge(other);
        assert!(!view.complete, "merging units does not complete a file");
        view.mark_complete();
        assert!(view.complete);
        let mut fresh = SyntaxView {
            file: "fixture.sh".to_string(),
            ..SyntaxView::default()
        };
        fresh.merge(view);
        assert!(fresh.complete, "completeness survives a merge");
    }
}
