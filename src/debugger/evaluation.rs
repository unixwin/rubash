//! Evaluation and write-back (debugger design 5.3/7.4/7.5/7.6;
//! PLAN row 8, WP-7).
//!
//! This module is the single owner of the debugger's evaluation
//! power rules — the same rules the section 5.5 capability face
//! and the section 6.3 diagnostic face speak in. No frontend and
//! no session-layer code builds a second rule set: a frontend
//! translates its input into an [`EvaluationRequest`], the
//! executor's consult-side machinery (in `executor::trap_exec`)
//! executes it through this module's classification, and the
//! [`EvaluationOutcome`] installed on the session is the only
//! result a frontend renders.
//!
//! The power rule (design 5.3/7.5):
//!
//! - **Explicit** evaluation — text the user actively typed at the
//!   debug console (`print`, `eval`) — is full power. A single
//!   expansion is expanded by the engine's own word expansion; a
//!   command list is executed by the engine's own execution, in
//!   the stopped frame, with the DEBUG path suppressed for its
//!   duration exactly as breakpoint conditions are (5.3).
//! - **Implicit** evaluation — hover, watch expressions, automatic
//!   display: anything the user did not just type — is strict.
//!   The text must be exactly one expansion, and that expansion
//!   must pass the whitelist in [`check_implicit`]. A rejected
//!   form is *skipped and marked*, never executed: the outcome
//!   carries the rejection as a section 6.3 diagnostic.
//!
//! Write-back (7.6) has no channel of its own here, by design:
//! an explicit evaluation whose text is an assignment *is* the
//! write-back, executed through the engine's real assignment
//! machinery, so attributes, array semantics, nameref resolution
//! and scope rules are the debuggee's own. Readonly and dynamic
//! names refuse exactly as the real path refuses them; the refusal
//! is the answer. Nothing in this module writes a variable.
//!
//! How the whitelist inspects (7.5: "parse with the engine's own
//! parser, inspect the AST, never regex"): the fragment is parsed
//! by the engine's parser and the command-level AST answers the
//! structural questions (one command, no connectors, no
//! assignments, no redirections, one word; process substitutions
//! and extglob patterns are parser records). Inside the one word,
//! this engine's parser deliberately keeps no expansion inventory
//! (the word records are zero-reader empties — see
//! `parser::assignment`); expansions are decomposed by the
//! expander at expansion time. The word walk below therefore
//! classifies expansion *spans* with the parser's own matched-pair
//! discipline (`parser::pathname_pattern::skip_nested_expansion`)
//! and the parser's own operator classification: parameter
//! operators by the subst.c operator classes the design lists,
//! arithmetic bodies by `parser::arithmetic_command`'s operator
//! scan — the same scan the `(( ))` command metadata is built
//! with, so the checker and the engine can never disagree about
//! what an assignment or update operator is.

use crate::parser::Ast;

/// Which power rule an evaluation runs under (design 5.3/7.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerMode {
    /// The user actively typed this text at the debug console:
    /// full power, side effects are the user's.
    Explicit,
    /// The frontend wants this value without the user having
    /// typed it (hover, watch, automatic display): whitelist
    /// only, rejections are skipped and marked.
    Implicit,
}

/// One evaluation the frontend asks the stopped session to run.
///
/// The stop loop returns this as its outcome; the executor runs
/// it and installs the [`EvaluationOutcome`] on the session, then
/// serves the same stop again — the stop loop never sees the
/// executor, and the executor never sees frontend input except
/// through this request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluationRequest {
    /// The power rule to evaluate under.
    pub mode: PowerMode,
    /// The text to evaluate: one expansion for `print`-shaped
    /// requests, an arbitrary command list for `eval`-shaped
    /// explicit requests.
    pub text: String,
    /// The frame the evaluation is addressed to, as an index into
    /// the stop's captured frame list (0 = innermost). Validated
    /// against the stop's snapshot; execution itself reads and
    /// writes the live frame context — the shell's scoping is
    /// dynamic, so the state a stopped frame's evaluation can
    /// touch is the live state's, which is exactly what the
    /// engine's own DEBUG-trap-time evaluation sees.
    pub frame: Option<usize>,
}

/// The diagnostic class of a failed or rejected evaluation
/// (design 6.3's diagnostic face).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticKind {
    /// The text did not parse.
    Parse,
    /// Execution of an explicit evaluation failed.
    Runtime,
    /// A builtin inside an explicit evaluation failed.
    Builtin,
    /// An expansion failed, or the implicit whitelist refused the
    /// form.
    Expansion,
}

/// A failure or rejection, in the section 6.3 shape: delivered as
/// text to the error sink and as values to API consumers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluationDiagnostic {
    /// What class of failure this is.
    pub kind: DiagnosticKind,
    /// The message, worded the way the engine words its own.
    pub message: String,
    /// The source the failing text came from, when known.
    pub source_name: Option<String>,
    /// The line within that source, when known.
    pub line: Option<usize>,
    /// The exit status the failure carries, when it has one.
    pub status: Option<i32>,
}

impl EvaluationDiagnostic {
    fn new(kind: DiagnosticKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            source_name: None,
            line: None,
            status: None,
        }
    }
}

/// What an evaluation produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvaluationResult {
    /// A single expansion was expanded; this is its rendered
    /// value (the `print` result).
    Value(String),
    /// A command list was executed; this is its exit status
    /// (the `eval` result).
    Status(i32),
    /// Implicit evaluation refused the form: nothing was
    /// executed, and the diagnostic is the mark the frontend
    /// shows in place of a value (design 7.5).
    Rejected(EvaluationDiagnostic),
    /// The evaluation was attempted and failed (parse error, or
    /// an explicit command list that errored).
    Failed(EvaluationDiagnostic),
}

/// The full record of one evaluation, installed on the session
/// for the frontend to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluationOutcome {
    /// The text that was evaluated, verbatim.
    pub text: String,
    /// The power rule it ran under.
    pub mode: PowerMode,
    /// What it produced.
    pub result: EvaluationResult,
}

/// The whitelist's rejection classes (design 7.5's list, plus the
/// structural classes a non-expansion carries). One class per
/// rejected form; the acceptance gate pins every class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectionKind {
    /// `$(...)` or the `${ ...; }` / `${| ...; }` nofork form.
    CommandSubstitution,
    /// Backquote substitution.
    BacktickSubstitution,
    /// `<(...)` / `>(...)`.
    ProcessSubstitution,
    /// `${x:=y}` / `${x=y}` — the expansion assigns.
    AssignmentExpansion,
    /// `${x:?y}` / `${x?y}` — the expansion can abort the shell.
    ErrorExpansion,
    /// `$((x = 1))` and the compound arithmetic assignments.
    ArithmeticAssignment,
    /// `$((x++))`, `$((x--))` and their prefix forms.
    ArithmeticUpdate,
    /// The text is an assignment statement, not an expansion.
    AssignmentForm,
    /// The text carries a redirection.
    RedirectForm,
    /// The text is not exactly one expansion (a command, a list,
    /// a compound form, several words).
    NotOneExpansion,
    /// A pathname pattern: its value depends on the filesystem,
    /// not on the stopped frame.
    PathnamePattern,
    /// A brace expansion: it yields several fields, not a value.
    BraceExpansion,
}

/// One whitelist rejection: the class and the offending source
/// fragment, so the frontend's mark can name what was skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhitelistRejection {
    /// Which rejection class fired.
    pub kind: RejectionKind,
    /// The offending fragment's source text.
    pub form: String,
}

impl WhitelistRejection {
    fn new(kind: RejectionKind, form: impl Into<String>) -> Self {
        Self {
            kind,
            form: form.into(),
        }
    }

    /// The human reason for the class, as the design states it.
    pub fn reason(&self) -> &'static str {
        match self.kind {
            RejectionKind::CommandSubstitution => "command substitution is not a read-only form",
            RejectionKind::BacktickSubstitution => "backquote substitution is not a read-only form",
            RejectionKind::ProcessSubstitution => "process substitution is not a read-only form",
            RejectionKind::AssignmentExpansion => {
                "the expansion assigns to the debuggee's variables"
            }
            RejectionKind::ErrorExpansion => "the expansion can abort the debuggee",
            RejectionKind::ArithmeticAssignment => {
                "the arithmetic assigns to the debuggee's variables"
            }
            RejectionKind::ArithmeticUpdate => "the arithmetic updates the debuggee's variables",
            RejectionKind::AssignmentForm => "an assignment is not an expansion",
            RejectionKind::RedirectForm => "a redirection is not an expansion",
            RejectionKind::NotOneExpansion => "implicit evaluation takes exactly one expansion",
            RejectionKind::PathnamePattern => {
                "pathname expansion depends on the filesystem, not the frame"
            }
            RejectionKind::BraceExpansion => "brace expansion yields more than one field",
        }
    }

    /// The rejection as a section 6.3 diagnostic — the mark an
    /// implicit evaluation leaves where a value would have been.
    pub fn diagnostic(&self) -> EvaluationDiagnostic {
        EvaluationDiagnostic::new(
            DiagnosticKind::Expansion,
            format!(
                "implicit evaluation skipped: {}: {}",
                self.form,
                self.reason()
            ),
        )
    }
}

/// Whether the parsed fragment is a compound or connected form
/// rather than one simple command.
fn is_compound_or_connected(cmd: &crate::parser::CommandNode) -> bool {
    cmd.pipe.is_some()
        || cmd.background
        || cmd.and_or.is_some()
        || cmd.inverted
        || cmd.subshell
        || cmd.subshell_end
        || cmd.pipeline_command.is_some()
        || cmd.and_or_list.is_some()
        || cmd.time_command.is_some()
        || cmd.background_command.is_some()
        || cmd.inverted_command.is_some()
        || cmd.for_command.is_some()
        || cmd.arithmetic_command.is_some()
        || cmd.if_command.is_some()
        || cmd.loop_command.is_some()
        || cmd.conditional_command.is_some()
        || cmd.subshell_command.is_some()
        || cmd.case_command.is_some()
        || cmd.select_command.is_some()
        || cmd.function_command.is_some()
        || cmd.brace_group.is_some()
        || cmd.coproc_command.is_some()
}

/// Whether the parsed fragment carries any redirection (a bare
/// redirection can create or truncate a file, so it is neither
/// an expansion nor a read).
fn has_redirection(cmd: &crate::parser::CommandNode) -> bool {
    !cmd.redirects.is_empty()
        || cmd.redirect_in.is_some()
        || cmd.redirect_out.is_some()
        || cmd.append.is_some()
        || cmd.redirect_err.is_some()
        || cmd.redirect_err_append.is_some()
        || cmd.heredoc.is_some()
        || !cmd.heredoc_redirects.is_empty()
        || cmd.here_string.is_some()
}

/// The structural shape both power rules share: exactly one
/// simple command of exactly one word, with no assignments and
/// no redirections. Returns the word (in the form the engine's
/// execution itself feeds to word expansion) when the fragment
/// has that shape.
///
/// This is a shape test only — it judges no power. The implicit
/// whitelist layers its classification on top of it; explicit
/// `print` routes through it to word expansion and everything
/// else to command execution.
pub fn single_expansion_word(ast: &Ast) -> Option<&str> {
    if ast.commands.len() != 1 {
        return None;
    }
    let cmd = &ast.commands[0];
    if !cmd.assignments.is_empty()
        || !cmd.compound_assignments.is_empty()
        || !cmd.array_element_assignments.is_empty()
        || has_redirection(cmd)
        || is_compound_or_connected(cmd)
        || cmd.words.len() != 1
    {
        return None;
    }
    cmd.words.first().map(String::as_str)
}

/// The raw source spelling of the fragment's single word (the
/// text the whitelist walks; the parsed `words` form carries the
/// lexer's in-band quote markers, the metadata `raw` is the text
/// as typed).
fn single_word_raw(ast: &Ast) -> Option<&str> {
    let cmd = ast.commands.first()?;
    match cmd.word_metadata.first() {
        Some(meta) if meta.word_index == 0 => Some(meta.raw.as_str()),
        _ => cmd.words.first().map(String::as_str),
    }
}

/// The implicit whitelist (design 7.5): the fragment must be one
/// expansion of a read-only form. On success, returns the word
/// to expand (the same string [`single_expansion_word`] yields).
/// On failure, the rejection names the first offending form —
/// the evaluation is skipped, never executed.
///
/// Allowed: plain and positional parameters, array subscripts,
/// `${!x}` indirection, casemod, `${x@Q}` and the other `@`
/// transforms, pattern and substring operations, assignment-free
/// arithmetic — including those forms nested inside an allowed
/// form's operand. Rejected: everything in [`RejectionKind`].
pub fn check_implicit<'a>(ast: &'a Ast, source_text: &str) -> Result<&'a str, WhitelistRejection> {
    if ast.commands.len() != 1 {
        return Err(WhitelistRejection::new(
            RejectionKind::NotOneExpansion,
            source_text.trim(),
        ));
    }
    let cmd = &ast.commands[0];
    if !cmd.assignments.is_empty()
        || !cmd.compound_assignments.is_empty()
        || !cmd.array_element_assignments.is_empty()
    {
        return Err(WhitelistRejection::new(
            RejectionKind::AssignmentForm,
            source_text.trim(),
        ));
    }
    if has_redirection(cmd) {
        return Err(WhitelistRejection::new(
            RejectionKind::RedirectForm,
            source_text.trim(),
        ));
    }
    if is_compound_or_connected(cmd) || cmd.words.len() != 1 {
        return Err(WhitelistRejection::new(
            RejectionKind::NotOneExpansion,
            source_text.trim(),
        ));
    }
    if let Some(subst) = cmd.process_substitutions.first() {
        return Err(WhitelistRejection::new(
            RejectionKind::ProcessSubstitution,
            format!("{}({})", subst.operator, subst.source),
        ));
    }
    if !cmd.extglob_patterns.is_empty() {
        return Err(WhitelistRejection::new(
            RejectionKind::PathnamePattern,
            source_text.trim(),
        ));
    }
    let raw = single_word_raw(ast).unwrap_or("");
    check_word_text(raw)?;
    Ok(single_expansion_word(ast).unwrap_or(""))
}

/// The parse-error marker the parser stamps on a failed
/// fragment (`__RUBASH_PARSE_ERROR__`, message as the value), as
/// a Parse diagnostic. Both power rules consult it before
/// anything executes: a fragment that did not parse is a failed
/// evaluation, never a command.
pub fn parse_failure(ast: &Ast) -> Option<EvaluationDiagnostic> {
    for cmd in &ast.commands {
        if let Some((_, message)) = cmd
            .assignments
            .iter()
            .find(|(name, _)| name == "__RUBASH_PARSE_ERROR__")
        {
            let mut diagnostic = EvaluationDiagnostic::new(DiagnosticKind::Parse, message.clone());
            diagnostic.line = cmd.line;
            return Some(diagnostic);
        }
    }
    None
}

/// Classify one word's text: every expansion span in
/// expansion-active territory must be a read-only form.
fn check_word_text(text: &str) -> Result<(), WhitelistRejection> {
    let chars: Vec<char> = text.chars().collect();
    check_word_chars(&chars, text)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuoteState {
    Unquoted,
    Single,
    Double,
}

/// Walk one word (or one operand fragment of one) in
/// expansion-active territory. `whole` is the text diagnostics
/// name when the offending form is the word itself (a glob).
fn check_word_chars(chars: &[char], whole: &str) -> Result<(), WhitelistRejection> {
    let mut state = QuoteState::Unquoted;
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        match state {
            QuoteState::Single => {
                if ch == '\'' {
                    state = QuoteState::Unquoted;
                }
                i += 1;
            }
            QuoteState::Unquoted | QuoteState::Double => match ch {
                // An escape quotes its character in both states
                // (in double quotes it only escapes $, `, ", \,
                // newline — skipping the next character is exact
                // for those and inert for the rest).
                '\\' => i += 2,
                '\'' if state == QuoteState::Unquoted => {
                    state = QuoteState::Single;
                    i += 1;
                }
                '"' => {
                    state = if state == QuoteState::Double {
                        QuoteState::Unquoted
                    } else {
                        QuoteState::Double
                    };
                    i += 1;
                }
                '$' => {
                    i = check_dollar(chars, i, whole)?;
                }
                '`' => {
                    let end = crate::parser::pathname_pattern::skip_nested_expansion(chars, i)
                        .unwrap_or(chars.len());
                    return Err(WhitelistRejection::new(
                        RejectionKind::BacktickSubstitution,
                        chars[i..end].iter().collect::<String>(),
                    ));
                }
                '<' | '>' if state == QuoteState::Unquoted && chars.get(i + 1) == Some(&'(') => {
                    let end = span_end(chars, i + 1);
                    return Err(WhitelistRejection::new(
                        RejectionKind::ProcessSubstitution,
                        chars[i..end].iter().collect::<String>(),
                    ));
                }
                '*' | '?' | '[' if state == QuoteState::Unquoted => {
                    return Err(WhitelistRejection::new(
                        RejectionKind::PathnamePattern,
                        whole,
                    ));
                }
                // Extglob groups are pathname patterns too (the
                // parser's own extglob record is checked at the
                // command level as well; this arm keeps operand
                // fragments, which have no AST, covered).
                '@' | '+' if state == QuoteState::Unquoted && chars.get(i + 1) == Some(&'(') => {
                    return Err(WhitelistRejection::new(
                        RejectionKind::PathnamePattern,
                        whole,
                    ));
                }
                '{' if state == QuoteState::Unquoted => {
                    if let Some(end) = matched_span_end(chars, i, '{', '}') {
                        if brace_body_expands(&chars[i + 1..end - 1]) {
                            return Err(WhitelistRejection::new(
                                RejectionKind::BraceExpansion,
                                chars[i..end].iter().collect::<String>(),
                            ));
                        }
                    }
                    i += 1;
                }
                _ => i += 1,
            },
        }
    }
    Ok(())
}

/// Classify the `$` construct at `chars[i]`; returns the index
/// just past it.
fn check_dollar(chars: &[char], i: usize, whole: &str) -> Result<usize, WhitelistRejection> {
    match chars.get(i + 1) {
        None => Ok(i + 1),
        Some('(') => {
            let end = crate::parser::pathname_pattern::skip_nested_expansion(chars, i)
                .unwrap_or(chars.len());
            if chars.get(i + 2) == Some(&'(') {
                // Arithmetic expansion: classify the body.
                let body_end = if end >= i + 5 && chars[end - 1] == ')' && chars[end - 2] == ')' {
                    end - 2
                } else {
                    end
                };
                let form: String = chars[i..end].iter().collect();
                if i + 3 <= body_end {
                    check_arithmetic_body(&chars[i + 3..body_end], &form)?;
                }
                Ok(end)
            } else {
                Err(WhitelistRejection::new(
                    RejectionKind::CommandSubstitution,
                    chars[i..end].iter().collect::<String>(),
                ))
            }
        }
        Some('{') => {
            let end = crate::parser::pathname_pattern::skip_nested_expansion(chars, i)
                .unwrap_or(chars.len());
            let form: String = chars[i..end].iter().collect();
            let body = if end >= i + 3 && chars[end - 1] == '}' {
                &chars[i + 2..end - 1]
            } else {
                &chars[i + 2..end]
            };
            // The nofork command substitutions (parse.y FUNSUB):
            // `${ cmd; }` and `${| cmd; }` execute.
            if matches!(body.first(), Some('|') | Some(' ') | Some('\t')) {
                return Err(WhitelistRejection::new(
                    RejectionKind::CommandSubstitution,
                    form,
                ));
            }
            classify_braced_body(body, &form)?;
            Ok(end)
        }
        Some('\'') => {
            // $'...': ANSI-C quoting — inert literal text.
            let mut j = i + 2;
            while j < chars.len() {
                match chars[j] {
                    '\\' => j += 2,
                    '\'' => return Ok(j + 1),
                    _ => j += 1,
                }
            }
            Ok(chars.len())
        }
        // $"...": locale string — the quote toggles the walk's
        // Double state on the next iteration, expansions inside
        // stay expansion-active.
        Some('"') => Ok(i + 1),
        Some('`') => {
            // The backquote opens on the next iteration and is
            // rejected there.
            Ok(i + 1)
        }
        Some(c) if is_name_start(*c) => {
            let mut j = i + 1;
            while j < chars.len() && is_name_char(chars[j]) {
                j += 1;
            }
            Ok(j)
        }
        Some(c) if c.is_ascii_digit() => {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            Ok(j)
        }
        Some(c) if is_special_parameter(*c) => Ok(i + 2),
        _ => Ok(i + 1),
    }
}

/// Classify one `${...}` body: name (with length marker and
/// indirection), optional subscript, and the operator class.
fn classify_braced_body(body: &[char], form: &str) -> Result<(), WhitelistRejection> {
    let mut rest = body;
    // Length form: ${#name}, ${#name[sub]}, ${#@}...
    if rest.len() > 1 && rest[0] == '#' && starts_name(&rest[1..]) {
        rest = &rest[1..];
    }
    // Indirection: ${!name}, ${!name[@]}; the name-list forms
    // ${!prefix*} / ${!prefix@} end at the marker below.
    if rest.len() > 1 && rest[0] == '!' {
        rest = &rest[1..];
    }
    if rest.is_empty() {
        return Ok(());
    }
    // The name.
    let first = rest[0];
    let name_end = if first.is_ascii_digit() {
        let mut j = 0;
        while j < rest.len() && rest[j].is_ascii_digit() {
            j += 1;
        }
        j
    } else if is_name_start(first) {
        let mut j = 0;
        while j < rest.len() && is_name_char(rest[j]) {
            j += 1;
        }
        j
    } else if is_special_parameter(first) {
        1
    } else {
        // No name the grammar admits: scan the whole body for
        // the writing operators at face value (a malformed form
        // cannot smuggle a write past the scan), and let the
        // expansion itself report the bad substitution.
        return scan_body_operators(rest, form);
    };
    rest = &rest[name_end..];
    // Optional subscript: its text is arithmetic context (plus
    // expansion spans of its own).
    if rest.first() == Some(&'[') {
        let end = matched_span_end(rest, 0, '[', ']').unwrap_or(rest.len());
        let inner = if end >= 2 && rest[end - 1] == ']' {
            &rest[1..end - 1]
        } else {
            &rest[1..end]
        };
        if inner != ['@'] && inner != ['*'] {
            check_arithmetic_body(inner, form)?;
        }
        rest = &rest[end..];
    }
    if rest.is_empty() {
        return Ok(());
    }
    // The operator class (subst.c's parameter_brace_expand arms).
    match rest[0] {
        ':' if rest.len() >= 2 => match rest[1] {
            '-' | '+' => check_word_chars(&rest[2..], form),
            '=' => Err(WhitelistRejection::new(
                RejectionKind::AssignmentExpansion,
                form,
            )),
            '?' => Err(WhitelistRejection::new(RejectionKind::ErrorExpansion, form)),
            // Substring: the offset/length text is arithmetic.
            _ => check_arithmetic_body(&rest[1..], form),
        },
        '-' | '+' => check_word_chars(&rest[1..], form),
        '=' => Err(WhitelistRejection::new(
            RejectionKind::AssignmentExpansion,
            form,
        )),
        '?' => Err(WhitelistRejection::new(RejectionKind::ErrorExpansion, form)),
        '#' | '%' => {
            let op_len = if rest.get(1) == Some(&rest[0]) { 2 } else { 1 };
            check_word_chars(&rest[op_len..], form)
        }
        '/' => check_word_chars(&rest[1..], form),
        '^' | ',' | '~' => {
            let op_len = if rest.get(1) == Some(&rest[0]) { 2 } else { 1 };
            let operand = &rest[op_len..];
            if operand.is_empty() {
                Ok(())
            } else {
                check_word_chars(operand, form)
            }
        }
        // `@` transforms are pure string operations. Anything
        // else names no form with power; the expansion itself
        // owns reporting a bad substitution.
        _ => Ok(()),
    }
}

/// The operator scan for a `${...}` body with no admissible
/// name: only the writing operators can hide there.
fn scan_body_operators(body: &[char], form: &str) -> Result<(), WhitelistRejection> {
    let mut i = 0;
    while i < body.len() {
        if body[i] == ':' && matches!(body.get(i + 1), Some('=') | Some('?')) {
            return Err(WhitelistRejection::new(
                if body[i + 1] == '=' {
                    RejectionKind::AssignmentExpansion
                } else {
                    RejectionKind::ErrorExpansion
                },
                form,
            ));
        }
        if body[i] == '=' || body[i] == '?' {
            // A leading or bare operator with no name in front
            // of it is bad-substitution territory — unless the
            // `=` is the whole body after a name we failed to
            // read, which the caller already handled. Stay
            // conservative only for the unmistakable forms.
            if i == 0 {
                return Err(WhitelistRejection::new(
                    if body[i] == '=' {
                        RejectionKind::AssignmentExpansion
                    } else {
                        RejectionKind::ErrorExpansion
                    },
                    form,
                ));
            }
        }
        i += 1;
    }
    Ok(())
}

/// Classify one arithmetic body (a `$((...))` body, a
/// subscript, or substring offset text): expansion spans inside
/// it classify recursively by the word rules, and the operator
/// inventory — taken with the parser's own arithmetic operator
/// scan over the text with expansion spans and quoted string
/// literals blanked (their contents are data, never operators:
/// `arr["a=b"]` reads a key, it does not assign) — must contain
/// no assignment and no update operator.
fn check_arithmetic_body(chars: &[char], form: &str) -> Result<(), WhitelistRejection> {
    let mut blanked: Vec<char> = chars.to_vec();
    let mut state = QuoteState::Unquoted;
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        match state {
            QuoteState::Single => {
                blanked[i] = ' ';
                if ch == '\'' {
                    state = QuoteState::Unquoted;
                }
                i += 1;
            }
            QuoteState::Unquoted | QuoteState::Double => match ch {
                '\\' => i += 2,
                '\'' if state == QuoteState::Unquoted => {
                    blanked[i] = ' ';
                    state = QuoteState::Single;
                    i += 1;
                }
                '"' => {
                    blanked[i] = ' ';
                    state = if state == QuoteState::Double {
                        QuoteState::Unquoted
                    } else {
                        QuoteState::Double
                    };
                    i += 1;
                }
                '$' => {
                    let next = check_dollar(chars, i, form)?;
                    for slot in blanked.iter_mut().take(next).skip(i) {
                        *slot = ' ';
                    }
                    i = next;
                }
                '`' => {
                    let end = crate::parser::pathname_pattern::skip_nested_expansion(chars, i)
                        .unwrap_or(chars.len());
                    return Err(WhitelistRejection::new(
                        RejectionKind::BacktickSubstitution,
                        chars[i..end].iter().collect::<String>(),
                    ));
                }
                _ if state == QuoteState::Double => {
                    blanked[i] = ' ';
                    i += 1;
                }
                _ => i += 1,
            },
        }
    }
    let text: String = blanked.iter().collect();
    for operator in crate::parser::arithmetic_command::arithmetic_operators(&text) {
        if crate::parser::arithmetic_command::is_arithmetic_assignment_operator(&operator.text) {
            return Err(WhitelistRejection::new(
                RejectionKind::ArithmeticAssignment,
                form,
            ));
        }
        if operator.text == "++" || operator.text == "--" {
            return Err(WhitelistRejection::new(
                RejectionKind::ArithmeticUpdate,
                form,
            ));
        }
    }
    Ok(())
}

/// The end index (just past the closer) of a bracketed span
/// opening at `chars[start]`, with nesting, quoting, and nested
/// expansion spans skipped the parser's own way.
fn matched_span_end(chars: &[char], start: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = start;
    while i < chars.len() {
        if let Some(next) = crate::parser::pathname_pattern::skip_nested_expansion(chars, i) {
            i = next;
            continue;
        }
        match chars[i] {
            '\\' => i += 2,
            '\'' => {
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    i += 1;
                }
                i += 1;
            }
            '"' => {
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i += 1;
            }
            c if c == open => {
                depth += 1;
                i += 1;
            }
            c if c == close => {
                depth -= 1;
                i += 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => i += 1,
        }
    }
    None
}

/// The end index just past a `)` span opening at `chars[start]`
/// (a process substitution's body).
fn span_end(chars: &[char], start: usize) -> usize {
    matched_span_end(chars, start, '(', ')').unwrap_or(chars.len())
}

/// The end index just past a `{` span opening at `chars[start]`.
fn brace_span_end(chars: &[char], start: usize) -> Option<usize> {
    matched_span_end(chars, start, '{', '}')
}

/// Whether a `{...}` body is a brace expansion (a top-level
/// comma list or a `..` range), as opposed to a literal brace
/// the shell leaves alone.
fn brace_body_expands(body: &[char]) -> bool {
    let mut depth = 0usize;
    let mut i = 0;
    while i < body.len() {
        if let Some(next) = crate::parser::pathname_pattern::skip_nested_expansion(body, i) {
            i = next;
            continue;
        }
        match body[i] {
            '\\' => i += 2,
            '{' => {
                depth += 1;
                i += 1;
            }
            '}' => {
                depth = depth.saturating_sub(1);
                i += 1;
            }
            ',' if depth == 0 => return true,
            '.' if depth == 0 && body.get(i + 1) == Some(&'.') => return true,
            _ => i += 1,
        }
    }
    false
}

fn is_name_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_name_char(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

/// The one-character special parameters (`$@ $* $# $? $- $$ $!
/// $0`), whose braced forms are plain reads.
fn is_special_parameter(ch: char) -> bool {
    matches!(ch, '@' | '*' | '#' | '?' | '-' | '$' | '!' | '0')
}

/// Whether `rest` begins with something a parameter name can
/// begin with (for the `${#...}` length strip: `${#}` itself is
/// the count read and is handled by the name parse).
fn starts_name(rest: &[char]) -> bool {
    match rest.first() {
        Some(c) => is_name_start(*c) || c.is_ascii_digit() || is_special_parameter(*c),
        None => false,
    }
}
