//! The single diagnostic voice for builtins (Stage-2 project ④, D3).
//!
//! The dispatcher constructs one `Diag` per builtin call and hands it
//! down to the processor and everything the processor calls; any code
//! that wants to emit a diagnostic takes it from the `&mut Diag` in
//! its hand — there is no other way to get a prolog. That is how the
//! A3-R1 escape closes structurally (Phase 4): a downstream machine
//! that takes `&mut Diag` cannot write to the wrong sink, because
//! the sink is bound once, at the dispatch point, to the command's
//! own stderr. Diagnostics are recorded in order and rendered by
//! `flush_with` after the call, with the prolog supplied there —
//! so a diagnostic-free call never builds one (E4, Phase 2).
//!
//! The rendering rules are ports of the GNU originals (behavior
//! ported, not code translated):
//!
//! * prolog — `Executor::diagnostic_prefix()`'s branches, which are
//!   themselves `builtins/common.c:83-96` (`builtin_error_prolog`)
//!   + `error.c:88-120` (`get_name_for_error`): interactive shells
//!   print only the shell basename; script/`-c` shells print
//!   `name: line N: ` (script file) or `name: line N: ` (`$0`).
//! * the thirteen skeletons — `builtins/common.c:139-286`
//!   (`sh_needarg`, `sh_neednumarg`, `sh_invalidopt`, …).
//! * `usage()` — `builtins/common.c:128-137` (`builtin_usage`):
//!   `{name}: usage: {short_doc}`, with NO prolog.
//! * `numeric_arg()` — `builtins/common.c:488-520`
//!   (`get_numeric_arg`) over `general.c`'s `valid_number` grammar
//!   (base-10 `strtoimax`, whole string modulo trailing whitespace,
//!   overflow invalid).
//! * `check_identifier()` — `general.c`'s `valid_identifier`
//!   (ASCII letter/`_` first, ASCII alnum/`_` after).
//! * `os_error()` — the errno text via `posix_errors::message`
//!   (never Rust's `(os error N)` form, never a localized
//!   `FormatMessage` string).
//!
//! Two deliberate adaptations from the design sketch (`DESIGN.md`
//! §4.3), both forced by the GNU evidence:
//!
//! * `DiagKind::InvalidOpt` / `NeedArg` carry the full two-character
//!   errstr (`"-z"`, `"+z"`) as a `String`, not a bare `char`:
//!   `bashgetopt.c:99-105` builds `errstr = {sign, char}` and
//!   `sh_invalidopt`/`sh_needarg` print it verbatim — the sign is
//!   part of the diagnostic (`shopt +z` reports `"+z"`, not `"-z"`).
//! * `Erange` / `NoJobs` / `Restricted` take `Option<String>` for
//!   the name: GNU's `sh_erange`/`sh_nojobs`/`sh_restricted` accept
//!   a NULL first argument and render the name-less form
//!   (`"%s out of range"`, `"no job control"`, `"restricted"`).
//!
//! Phase 0 lands the handle; no builtin is rewired onto it yet
//! (that is Phases 1–5). The `status` field records the exit status
//! the diagnostic implies (`EX_USAGE` for the getopt family); the
//! caller reads it after the call and acts on it.

use super::builtin_table::BuiltinRow;
use std::fmt;
use std::io::{self, Write};

/// The exit status for a usage error (`EX_USAGE`, `sysexits.h`).
pub const EX_USAGE: i32 = 2;
/// The exit status for a bad-usage numeric error (`EX_BADUSAGE`).
pub const EX_BADUSAGE: i32 = 2;

/// The diagnostic prolog: the shell name + the line segment, in
/// the shape `Executor::diagnostic_prefix()` already renders. The
/// dispatcher builds one at flush time (`Diag::flush_with`) and
/// hands it to the handle then — never at `Diag` construction,
/// so the handle itself does not borrow the executor and a
/// diagnostic-free call never pays the ~7µs build cost (E4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prolog {
    /// Whether the shell is interactive (no line segment then).
    pub interactive: bool,
    /// `base_pathname(shell_name)` — the interactive prolog name.
    pub shell_basename: String,
    /// `error_report_name()`: BASH_SOURCE[0], else `$0` verbatim,
    /// else `base_pathname(shell_name)`, else `"bash"`.
    pub error_name: String,
    /// `current_error_file_name()`: the innermost executing file.
    pub script_file: Option<String>,
    /// `__RUBASH_CURRENT_LINE` for the executing frame.
    pub line: Option<String>,
}

impl Prolog {
    /// Render the prolog (`builtins/common.c:83-96` shape):
    /// `"{name}: "` interactively, `"{script}: line {n}: "` for a
    /// script frame, `"{name}: line {n}: "` for `-c`, `"{name}: "`
    /// with no line otherwise. The trailing `": "` is part of the
    /// prolog; the command name follows from the `Diag`.
    pub fn render(&self) -> String {
        if self.interactive {
            return format!("{}: ", self.shell_basename);
        }
        match (&self.script_file, &self.line) {
            (Some(script), Some(line)) => format!("{script}: line {line}: "),
            (None, Some(line)) => format!("{}: line {line}: ", self.error_name),
            _ => format!("{}: ", self.error_name),
        }
    }
}

/// The deferred form of a `Prolog`, for the declare family's
/// in-context entry: the variable table is mutably borrowed for the
/// whole call there, so the composed `Prolog` (which reads that
/// same table) cannot be handed in by the dispatcher without paying
/// its ~7µs build on every call — and a closure over the executor
/// cannot build it either, since that borrow would coexist with
/// the `&mut` table borrow for the call's duration. The two fields
/// that do NOT derive from the table — the composed error file
/// name (a `bash_source_stack` product) and the invocation shape —
/// cross by value; every table-derived field is composed in
/// `build`, which runs only at a diagnostic site, so a
/// diagnostic-free declare/local call never pays for a prolog
/// (E4, the Phase-2 lesson — see `Diag`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrologParts {
    /// The executor flavor (`Executor::diagnostic_prolog`,
    /// deferred): the fields are composed at the dispatch point.
    Executor {
        /// `current_error_file_name()` at the dispatch point.
        script_file: Option<String>,
        /// `invocation_interactive()` at the dispatch point.
        interactive: bool,
    },
    /// The library flavor (the declare layer's own table-only
    /// composition, for the non-executor entry).
    Lib,
}

impl PrologParts {
    /// Compose the `Prolog` from the variable table, exactly as
    /// the eager builder of the same flavor composes it (the
    /// table-derived fields — the argv0 chain, the shell basename,
    /// the current line — are read here, at the diagnostic site;
    /// the declare family never writes those cells itself).
    pub fn build(&self, variables: &crate::shell::var_table::VarTable) -> Prolog {
        match self {
            PrologParts::Executor {
                script_file,
                interactive,
            } => Prolog {
                interactive: *interactive,
                shell_basename: super::Executor::interactive_shell_basename(&**variables),
                error_name: error_report_name(script_file, variables),
                script_file: script_file.clone(),
                line: variables.get("__RUBASH_CURRENT_LINE").cloned(),
            },
            PrologParts::Lib => {
                // rubash#3355: the lib flavor's error_name used to be
                // the hardcoded "rubash" brand, so declare-family
                // diagnostics never followed $0 — under an empty
                // argv[0] they printed "rubash: ..." instead of the
                // empty name GNU prints. Compose it from the same
                // get_name_for_error chain as the executor flavor.
                let script_file = variables
                    .get("__RUBASH_SCRIPT_NAME")
                    .cloned()
                    .or_else(|| std::env::var("__RUBASH_SCRIPT_NAME").ok());
                Prolog {
                    interactive: variables.get("__RUBASH_INTERACTIVE").is_some(),
                    shell_basename: variables
                        .get("__RUBASH_SHELL_NAME")
                        .cloned()
                        .unwrap_or_else(|| "rubash".to_string()),
                    error_name: error_report_name(&script_file, variables),
                    script_file,
                    line: variables
                        .get("__RUBASH_CURRENT_LINE")
                        .cloned()
                        .or_else(|| std::env::var("__RUBASH_CURRENT_LINE").ok()),
                }
            }
        }
    }
}

/// `Executor::error_report_name_with_file` over a variable table:
/// the error file name when it names one, else the raw argv0 keys
/// in order, else the shell basename. Shared with the lib-level
/// prolog builders (declare/set/unset/alias families) so their
/// diagnostics follow $0 like the executor's do.
pub(crate) fn error_report_name(
    file_name: &Option<String>,
    variables: &crate::shell::var_table::VarTable,
) -> String {
    if let Some(name) = file_name.as_ref().filter(|name| !name.is_empty()) {
        return name.clone();
    }
    // rubash#3355: same presence-based $0 step as
    // Executor::error_report_name_with_file — GNU error.c:90-120
    // get_name_for_error stops at dollar_vars[0] itself, so a
    // present-but-empty $0 (empty argv[0]) is the verdict, not a
    // reason to fall through to the shell basename or PROGRAM.
    for key in [
        "BASH_ARGV0",
        "__RUBASH_ARGV0_AFTER_UNSET",
        "__RUBASH_TOP_LEVEL_NAME",
    ] {
        if let Some(value) = variables.get(key) {
            return value.clone();
        }
    }
    super::Executor::interactive_shell_basename(&**variables)
}

/// The sink a `Diag` writes to: the command's own stderr (the
/// redirection-applied IO context the dispatcher binds).
pub trait DiagSink {
    fn write_str(&mut self, s: &str);
}

impl<T: Write> DiagSink for T {
    fn write_str(&mut self, s: &str) {
        let _ = self.write_all(s.as_bytes());
        let _ = self.flush();
    }
}

/// One of the standard diagnostic skeletons (`builtins/common.c`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagKind {
    /// `sh_invalidopt`: `"{errstr}: invalid option"` — the errstr
    /// is the sign + the one character (`"-z"`, `"+z"`).
    InvalidOpt(String),
    /// `sh_needarg`: `"{errstr}: option requires an argument"`.
    NeedArg(String),
    /// `sh_neednumarg`: `"{errstr}: numeric argument required"`.
    NeedNumArg(String),
    /// `sh_invalidnum`: `"{s}: invalid number"` (with the octal /
    /// hex variants for `0…` / `0x…` prefixes).
    InvalidNum(String),
    /// `sh_invalidid`: `` "`{s}': not a valid identifier" ``.
    InvalidId(String),
    /// `sh_readonly`: `"{s}: readonly variable"`.
    Readonly(String),
    /// `sh_erange`: `"{s}: {desc} out of range"`, or `"{desc} out
    /// of range"` when the name is absent.
    Erange(Option<String>, String),
    /// `sh_badpid`: `` "`{s}': not a pid or valid job spec" ``.
    BadPid(String),
    /// `sh_nojobs`: `"{s}: no job control"`, or `"no job control"`.
    NoJobs(Option<String>),
    /// `sh_restricted`: `"{s}: restricted"`, or `"restricted"`.
    Restricted(Option<String>),
    /// `no_args`: `"too many arguments"`.
    TooManyArgs,
    /// The no-arguments error for builtins that take none beyond
    /// what was given (`builtin_error("too many arguments")` is
    /// `TooManyArgs`; this is the bare `"no arguments"` shape used
    /// where a builtin requires at least one operand).
    NoArgs,
}

/// How far `numeric_arg`'s failure unwinds (`get_numeric_arg`'s
/// `fatal` parameter, `common.c:488-520`): `0` returns to the
/// caller, `1` exits the shell, `2` discards the current command.
/// The unwind itself is the caller's job (there is no `setjmp`
/// here); the level is carried so the caller can act on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumericFatal {
    /// `fatal == 0`: return `None`, no status set.
    Soft,
    /// `fatal == 1`: abort the shell (`EXITPROG`).
    ExitShell,
    /// `fatal == 2`: discard the current command (`DISCARD`).
    AbortCommand,
}

/// The single diagnostic voice for one builtin call.
///
/// A `Diag` records diagnostics in order and renders them when
/// the dispatcher flushes it (`flush_with`, after the call):
/// the prolog is supplied there, by the dispatcher, so a call
/// that emits no diagnostic never pays for one — building a
/// prolog costs ~7µs per call in a debug build (~11 HashMap
/// lookups + string clones), which the E4 gate cannot absorb on
/// the fast builtins (Stage-2 ④ Phase 2). The sink is bound
/// once, at the dispatch point, to the command's own stderr.
pub struct Diag<'a> {
    row: &'a BuiltinRow,
    sink: &'a mut dyn DiagSink,
    status: i32,
    /// Diagnostics recorded but not yet rendered, in order: the
    /// line body without the prolog (`{name}: {body}\n`), plus
    /// whether the prolog prefixes it — `usage()` lines never
    /// carry one (`builtin_usage` has no prolog, common.c:128).
    pending: Vec<(bool, String)>,
}

impl<'a> Diag<'a> {
    pub fn new(row: &'a BuiltinRow, sink: &'a mut dyn DiagSink) -> Self {
        Diag {
            row,
            sink,
            status: 0,
            pending: Vec::new(),
        }
    }

    /// The exit status implied by the diagnostics emitted so far
    /// (`EX_USAGE` once a getopt-family error has fired, else 0).
    pub fn status(&self) -> i32 {
        self.status
    }

    /// Whether any diagnostic is recorded and unflushed.
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Render every recorded diagnostic into the sink, in order,
    /// each prefixed with the prolog — except `usage()` lines,
    /// which never carry one. The prolog is built by the caller
    /// and supplied here, exactly once per flush; when nothing is
    /// pending the closure is never called, so a diagnostic-free
    /// call costs one `is_empty` check and nothing more. The
    /// dispatcher must call this after the call and before the
    /// sink is read; a `Diag` dropped with pending diagnostics
    /// loses them (the discard-sink predicates rely on that).
    pub fn flush_with(&mut self, prolog: impl FnOnce() -> Prolog) {
        if self.pending.is_empty() {
            return;
        }
        let head = prolog().render();
        self.flush_with_head(&head);
    }

    /// Render pending diagnostics with an already-rendered prolog
    /// head. The immediate writers (`err_readonly`, the declare
    /// layer's `bare_diag_line`) flush through here first so a
    /// diagnostic recorded earlier in the same call still precedes
    /// a later immediate line in the byte stream — without it the
    /// pending line would land after the immediate one at flush
    /// time (Stage-2 ④ Phase 4).
    pub fn flush_with_head(&mut self, head: &str) {
        for (with_prolog, body) in self.pending.drain(..) {
            if with_prolog {
                self.sink.write_str(&format!("{head}{body}"));
            } else {
                self.sink.write_str(&body);
            }
        }
    }

    /// The row this handle speaks for.
    pub fn row(&self) -> &'a BuiltinRow {
        self.row
    }

    /// The raw diagnostic sink, for diagnostics that carry their own
    /// prefix and cannot speak through the standard `{prolog}{name}:`
    /// skeleton — the readonly-variable bind errors (`err_readonly`,
    /// error.c:453), which are `{prefix}VAR: readonly variable` with
    /// no builtin name and a gnu_errfmt-aware prefix of their own.
    /// The caller must not write a standard builtin diagnostic
    /// through this handle (that is what `free` is for).
    ///
    /// Direct sink writes bypass the pending queue: they land in
    /// the buffer immediately, while recorded diagnostics land at
    /// flush time. A caller that mixes the two in one call must
    /// ensure no diagnostic is pending when it writes — in
    /// practice the only user (`pushd`'s readonly bind errors)
    /// runs only on paths where none can be: a failed scan
    /// returns before any bind, and a failed `chdir` returns
    /// before any bind, so the bind errors only ever fire on a
    /// clean-scan success path with an empty queue.
    pub fn sink(&mut self) -> &mut dyn DiagSink {
        self.sink
    }

    /// Render one standard skeleton: `{prolog}{name}: {body}\n`.
    /// The getopt family (`InvalidOpt` / `NeedArg` / `NeedNumArg`)
    /// also sets `EX_USAGE`, matching the `.def` loops' `default:`
    /// arm (`builtin_usage(); return (EX_USAGE);`). The line is
    /// recorded, not written: `flush_with` renders it (with the
    /// prolog) after the call, in order.
    pub fn error(&mut self, kind: DiagKind) {
        match kind {
            DiagKind::InvalidOpt(_) | DiagKind::NeedArg(_) | DiagKind::NeedNumArg(_) => {
                self.status = EX_USAGE;
            }
            _ => {}
        }
        self.report(kind);
    }

    /// The same skeleton written without touching the status: for
    /// diagnostics whose outcome the caller decides (`numeric_arg`'s
    /// fatality — GNU's `get_numeric_arg` prints `sh_neednumarg`
    /// and returns 0, and only the caller sets the status).
    pub fn report(&mut self, kind: DiagKind) {
        let body = match &kind {
            DiagKind::InvalidOpt(s) => format!("{s}: invalid option"),
            DiagKind::NeedArg(s) => format!("{s}: option requires an argument"),
            DiagKind::NeedNumArg(s) => format!("{s}: numeric argument required"),
            DiagKind::InvalidNum(s) => {
                let msg = if s.len() >= 2 && s.starts_with('0') && s.as_bytes()[1].is_ascii_digit()
                {
                    "invalid octal number"
                } else if s.starts_with("0x") {
                    "invalid hex number"
                } else {
                    "invalid number"
                };
                format!("{s}: {msg}")
            }
            DiagKind::InvalidId(s) => format!("`{s}': not a valid identifier"),
            DiagKind::Readonly(s) => format!("{s}: readonly variable"),
            DiagKind::Erange(name, desc) => match name {
                Some(n) => format!("{n}: {desc} out of range"),
                None => format!("{desc} out of range"),
            },
            DiagKind::BadPid(s) => format!("`{s}': not a pid or valid job spec"),
            DiagKind::NoJobs(name) => match name {
                Some(n) => format!("{n}: no job control"),
                None => "no job control".to_string(),
            },
            DiagKind::Restricted(name) => match name {
                Some(n) => format!("{n}: restricted"),
                None => "restricted".to_string(),
            },
            DiagKind::TooManyArgs => "too many arguments".to_string(),
            DiagKind::NoArgs => "no arguments".to_string(),
        };
        let line = format!("{}: {body}\n", self.row.name);
        self.pending.push((true, line));
    }

    /// `builtin_usage` (`common.c:128-137`): `{name}: usage:
    /// {short_doc}` — with NO prolog. A row with no help topic
    /// (`env`) prints nothing: there is no short doc to print.
    /// Recorded like any other line (in order); `flush_with`
    /// renders it without the prolog.
    pub fn usage(&mut self) {
        if let Some(short) = self.row.doc.short_doc() {
            let line = format!("{}: usage: {short}\n", self.row.name);
            self.pending.push((false, line));
        }
    }

    /// `get_numeric_arg` (`common.c:488-520`): read a numeric
    /// argument from the operand words. A leading `--` word is
    /// skipped (`ISOPTION(word, '-')`, `common.h:26` — exactly the
    /// two-character word, not any `-X` word); with no word the
    /// count defaults to 1. On an invalid number `sh_neednumarg`
    /// fires for the word and `None` is returned (the status is set
    /// to `EX_BADUSAGE` unless `fatal` is `Soft`). Extra words
    /// after the numeric one fire `TooManyArgs` and return `None` —
    /// GNU's `no_args` jumps, so the caller must not proceed.
    pub fn numeric_arg(&mut self, words: &[String], fatal: NumericFatal) -> Option<i64> {
        let mut words = words;
        if let Some(first) = words.first() {
            if first == "--" {
                words = &words[1..];
            }
        }
        let Some(arg) = words.first() else {
            return Some(1);
        };
        let Some(value) = valid_number(arg) else {
            self.report(DiagKind::NeedNumArg(arg.clone()));
            if fatal != NumericFatal::Soft {
                self.status = EX_BADUSAGE;
            }
            return None;
        };
        if words.len() > 1 {
            self.report(DiagKind::TooManyArgs);
            if fatal != NumericFatal::Soft {
                self.status = EX_BADUSAGE;
            }
            return None;
        }
        Some(value)
    }

    /// `sh_invalidid` (`general.c`'s `valid_identifier`): true
    /// when `name` is a valid identifier, else the diagnostic and
    /// false.
    pub fn check_identifier(&mut self, name: &str) -> bool {
        if valid_identifier(name) {
            true
        } else {
            self.error(DiagKind::InvalidId(name.to_string()));
            false
        }
    }

    /// An OS error on `ctx` (a path or operand): `{prolog}{name}:
    /// {ctx}: {strerror}\n`, with the canonical GNU strerror text
    /// (`posix_errors::message` — never `(os error N)`, never a
    /// localized message). The status is the caller's business.
    /// Recorded, not written (see `error`).
    pub fn os_error(&mut self, ctx: &str, err: &io::Error) {
        let text = crate::posix_errors::message(err);
        let line = format!("{}: {ctx}: {text}\n", self.row.name);
        self.pending.push((true, line));
    }

    /// `err_readonly` (`error.c:453`): the bind-layer diagnostic
    /// `{prefix}VAR: readonly variable` with no builtin name and a
    /// gnu_errfmt-aware prefix of its own. Written immediately to
    /// the sink like `sink()` lines — the caller supplies the
    /// already-rendered prefix (the assignment machinery's
    /// `error_prefix`, the declare layer's flush prolog) and must
    /// flush pending diagnostics first when order matters (see
    /// `bare_diag_line` in `builtins::declare`).
    pub fn err_readonly(&mut self, error_prefix: &str, name: &str) {
        // Flush first: this line writes immediately, so a diagnostic
        // recorded earlier in the same call must precede it.
        self.flush_with_head(error_prefix);
        let line = format!("{error_prefix}{name}: readonly variable\n");
        self.sink.write_str(&line);
    }

    /// A non-standard diagnostic body through the same prolog and
    /// sink: `{prolog}{name}: {message}\n`. Recorded, not written
    /// (see `error`).
    pub fn free(&mut self, args: fmt::Arguments<'_>) {
        let line = format!("{}: {}\n", self.row.name, args);
        self.pending.push((true, line));
    }
}

/// `valid_number` (`general.c`): base-10 `strtoimax` over the
/// whole string modulo trailing whitespace; overflow invalid.
/// The whitespace strtoimax skips is the C `isspace` set — space,
/// \t, \n, \v, \f, \r. `str::trim_ascii` lacks the vertical tab
/// (0x0B), so a `\v`-padded number GNU accepts was rejected
/// (rubash#2958).
pub(crate) fn valid_number(s: &str) -> Option<i64> {
    // The word is judged in its decoded form: the carrier bytes
    // {0x0C, 0x11, 0x13} arrive as raw-byte marker pairs from
    // expansion (`is_carrier_byte`, substitution_metadata.rs),
    // and GNU — which has no carrier — judges the byte itself,
    // so an expansion-produced \f counts as the form feed it
    // carries (rubash#2958).
    let decoded;
    let s = if s.contains(
        char::from_u32(crate::executor::markers::RAW_BYTE_MARKER_ESCAPE)
            .expect("sentinel is valid"),
    ) {
        decoded = crate::executor::substitution_metadata::decode_raw_byte_markers_to_byte_chars(s);
        decoded.as_str()
    } else {
        s
    };
    let trimmed =
        s.trim_matches(|c: char| matches!(c, ' ' | '\t' | '\n' | '\u{b}' | '\u{c}' | '\r'));
    if trimmed.is_empty() {
        return None;
    }
    trimmed.parse::<i64>().ok()
}

/// `valid_identifier` (`general.c`): non-empty, ASCII letter or
/// `_` first, ASCII alnum or `_` after.
pub(crate) fn valid_identifier(name: &str) -> bool {
    let mut chars = name.bytes();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

#[cfg(test)]
mod tests {
    use super::super::builtin_table::{builtin_table_row, BuiltinRow};
    use super::*;

    fn prolog_script() -> Prolog {
        Prolog {
            interactive: false,
            shell_basename: "bash".to_string(),
            error_name: "bash".to_string(),
            script_file: Some("t.sh".to_string()),
            line: Some("1".to_string()),
        }
    }

    fn prolog_interactive() -> Prolog {
        Prolog {
            interactive: true,
            shell_basename: "bash".to_string(),
            error_name: "bash".to_string(),
            script_file: None,
            line: None,
        }
    }

    fn prolog_dash_c() -> Prolog {
        Prolog {
            interactive: false,
            shell_basename: "bash".to_string(),
            error_name: "bash".to_string(),
            script_file: None,
            line: Some("1".to_string()),
        }
    }

    fn cd_row() -> &'static BuiltinRow {
        builtin_table_row("cd").unwrap()
    }

    #[test]
    fn prolog_renders_all_four_branches() {
        assert_eq!(prolog_script().render(), "t.sh: line 1: ");
        assert_eq!(prolog_interactive().render(), "bash: ");
        assert_eq!(prolog_dash_c().render(), "bash: line 1: ");
        let no_line = Prolog {
            interactive: false,
            shell_basename: "bash".to_string(),
            error_name: "bash".to_string(),
            script_file: None,
            line: None,
        };
        assert_eq!(no_line.render(), "bash: ");
    }

    #[test]
    fn error_renders_skeletons_and_sets_ex_usage() {
        let mut sink = Vec::new();
        let mut diag = Diag::new(cd_row(), &mut sink);
        diag.error(DiagKind::InvalidOpt("-z".to_string()));
        diag.error(DiagKind::NeedArg("-p".to_string()));
        diag.error(DiagKind::NeedNumArg("-n".to_string()));
        diag.error(DiagKind::InvalidNum("08".to_string()));
        diag.error(DiagKind::InvalidNum("0xzz".to_string()));
        diag.error(DiagKind::InvalidNum("zz".to_string()));
        diag.error(DiagKind::InvalidId("1x".to_string()));
        diag.error(DiagKind::Readonly("HOME".to_string()));
        diag.error(DiagKind::Erange(
            Some("ulimit".to_string()),
            "file size".to_string(),
        ));
        diag.error(DiagKind::Erange(None, "argument".to_string()));
        diag.error(DiagKind::BadPid("%99".to_string()));
        diag.error(DiagKind::NoJobs(None));
        diag.error(DiagKind::Restricted(Some("set".to_string())));
        diag.error(DiagKind::TooManyArgs);
        assert_eq!(diag.status(), EX_USAGE);
        diag.flush_with(|| prolog_script());
        let out = String::from_utf8(sink).unwrap();
        let expected = concat!(
            "t.sh: line 1: cd: -z: invalid option\n",
            "t.sh: line 1: cd: -p: option requires an argument\n",
            "t.sh: line 1: cd: -n: numeric argument required\n",
            "t.sh: line 1: cd: 08: invalid octal number\n",
            "t.sh: line 1: cd: 0xzz: invalid hex number\n",
            "t.sh: line 1: cd: zz: invalid number\n",
            "t.sh: line 1: cd: `1x': not a valid identifier\n",
            "t.sh: line 1: cd: HOME: readonly variable\n",
            "t.sh: line 1: cd: ulimit: file size out of range\n",
            "t.sh: line 1: cd: argument out of range\n",
            "t.sh: line 1: cd: `%99': not a pid or valid job spec\n",
            "t.sh: line 1: cd: no job control\n",
            "t.sh: line 1: cd: set: restricted\n",
            "t.sh: line 1: cd: too many arguments\n",
        );
        assert_eq!(out, expected);
    }

    #[test]
    fn error_plus_sign_is_kept() {
        let mut sink = Vec::new();
        let mut diag = Diag::new(builtin_table_row("shopt").unwrap(), &mut sink);
        diag.error(DiagKind::InvalidOpt("+z".to_string()));
        diag.flush_with(|| prolog_script());
        assert_eq!(
            String::from_utf8(sink).unwrap(),
            "t.sh: line 1: shopt: +z: invalid option\n"
        );
    }

    #[test]
    fn usage_has_no_prolog_and_env_prints_nothing() {
        let mut sink = Vec::new();
        let mut diag = Diag::new(cd_row(), &mut sink);
        diag.usage();
        diag.flush_with(|| prolog_script());
        assert_eq!(
            String::from_utf8(sink).unwrap(),
            "cd: usage: cd [-L|[-P [-e]]] [-@] [dir]\n"
        );
        let mut sink = Vec::new();
        let mut diag = Diag::new(builtin_table_row("env").unwrap(), &mut sink);
        diag.usage();
        diag.flush_with(|| prolog_script());
        assert!(sink.is_empty(), "env has no short doc to print");
    }

    #[test]
    fn numeric_arg_ports_get_numeric_arg() {
        let w = |ss: &[&str]| ss.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let mut sink = Vec::new();
        let mut diag = Diag::new(builtin_table_row("break").unwrap(), &mut sink);
        assert_eq!(diag.numeric_arg(&[], NumericFatal::Soft), Some(1));
        assert_eq!(diag.numeric_arg(&w(&["3"]), NumericFatal::Soft), Some(3));
        assert_eq!(diag.numeric_arg(&w(&["-2"]), NumericFatal::Soft), Some(-2));
        // A leading "--" is skipped (ISOPTION), so no count follows.
        assert_eq!(diag.numeric_arg(&w(&["--"]), NumericFatal::Soft), Some(1));
        assert_eq!(
            diag.numeric_arg(&w(&["--", "3"]), NumericFatal::Soft),
            Some(3)
        );
        // "-n" is NOT skipped (ISOPTION means exactly "--"): it is
        // the would-be number, and it is not one.
        assert_eq!(diag.numeric_arg(&w(&["-n"]), NumericFatal::Soft), None);
        assert_eq!(
            diag.numeric_arg(&w(&[" 12 "]), NumericFatal::Soft),
            Some(12)
        );
        assert_eq!(
            diag.numeric_arg(&w(&["9999999999999999999999"]), NumericFatal::Soft),
            None
        );
        assert_eq!(diag.numeric_arg(&w(&["2", "3"]), NumericFatal::Soft), None);
        diag.flush_with(|| prolog_script());
        let out = String::from_utf8(sink).unwrap();
        assert!(out.contains("break: 9999999999999999999999: numeric argument required\n"));
        assert!(out.contains("break: too many arguments\n"));
    }

    #[test]
    fn valid_number_uses_the_c_isspace_set() {
        // general.c valid_number via strtoimax: the padding set is
        // C isspace — vertical tab (0x0B) and form feed (0x0C)
        // included (rubash#2958; `str::trim_ascii` lacks \v, which
        // is why this is pinned at the core, not per builtin).
        assert_eq!(valid_number("\u{b}42"), Some(42));
        assert_eq!(valid_number("42\u{b}"), Some(42));
        assert_eq!(valid_number("\u{c}7"), Some(7));
        // The carrier-byte form of the same padding: 0x0C rides
        // expansion as the raw-byte marker pair U+E000 U+E00D and
        // must judge as the form feed it carries.
        assert_eq!(valid_number("\u{E000}\u{E00D}7"), Some(7));
        assert_eq!(valid_number(" \t\n\u{b}\u{c}\r-13 \u{b}"), Some(-13));
        assert_eq!(valid_number("\u{b}"), None);
        assert_eq!(valid_number("4\u{b}2"), None);
    }

    #[test]
    fn numeric_arg_soft_leaves_status_clear() {
        let w = |ss: &[&str]| ss.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let mut sink = Vec::new();
        let mut diag = Diag::new(cd_row(), &mut sink);
        assert_eq!(diag.numeric_arg(&w(&["xx"]), NumericFatal::Soft), None);
        assert_eq!(diag.status(), 0, "fatal==0 sets no status");
    }

    #[test]
    fn check_identifier_uses_valid_identifier() {
        let mut sink = Vec::new();
        let mut diag = Diag::new(cd_row(), &mut sink);
        assert!(diag.check_identifier("HOME"));
        assert!(diag.check_identifier("_x1"));
        assert!(!diag.check_identifier("1x"));
        assert!(!diag.check_identifier(""));
        assert!(!diag.check_identifier("a-b"));
        assert!(!diag.check_identifier("é"));
        diag.flush_with(|| prolog_script());
        assert_eq!(
            String::from_utf8(sink).unwrap(),
            concat!(
                "t.sh: line 1: cd: `1x': not a valid identifier\n",
                "t.sh: line 1: cd: `': not a valid identifier\n",
                "t.sh: line 1: cd: `a-b': not a valid identifier\n",
                "t.sh: line 1: cd: `é': not a valid identifier\n",
            )
        );
    }

    #[test]
    fn os_error_has_no_os_error_suffix() {
        let mut sink = Vec::new();
        let mut diag = Diag::new(cd_row(), &mut sink);
        diag.os_error("/nonexistent", &io::Error::from_raw_os_error(2));
        diag.flush_with(|| prolog_script());
        let out = String::from_utf8(sink).unwrap();
        assert_eq!(
            out,
            "t.sh: line 1: cd: /nonexistent: No such file or directory\n"
        );
        assert!(!out.contains("os error"), "the (os error N) form is banned");
    }

    #[test]
    fn free_uses_the_same_prolog_and_sink() {
        let mut sink = Vec::new();
        let mut diag = Diag::new(cd_row(), &mut sink);
        diag.free(format_args!("cannot cd to {}", "/tmp"));
        diag.flush_with(|| prolog_interactive());
        assert_eq!(
            String::from_utf8(sink).unwrap(),
            "bash: cd: cannot cd to /tmp\n"
        );
    }
}
