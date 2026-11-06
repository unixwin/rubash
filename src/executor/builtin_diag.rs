//! The single diagnostic voice for builtins (Stage-2 project ④, D3).
//!
//! The dispatcher constructs one `Diag` per builtin call and hands it
//! down to the processor and everything the processor calls; any code
//! that wants to emit a diagnostic takes it from the `&mut Diag` in
//! its hand — there is no other way to get a prolog. That is how the
//! A3-R1 escape closes structurally (Phase 4): a downstream machine
//! that takes `&mut Diag` cannot write to the wrong sink, because
//! the sink is bound once, at the dispatch point, to the command's
//! own stderr.
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
/// data is captured up front so the handle does not borrow the
/// executor.
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
pub struct Diag<'a> {
    prolog: Prolog,
    row: &'a BuiltinRow,
    sink: &'a mut dyn DiagSink,
    status: i32,
}

impl<'a> Diag<'a> {
    pub fn new(prolog: Prolog, row: &'a BuiltinRow, sink: &'a mut dyn DiagSink) -> Self {
        Diag {
            prolog,
            row,
            sink,
            status: 0,
        }
    }

    /// The exit status implied by the diagnostics emitted so far
    /// (`EX_USAGE` once a getopt-family error has fired, else 0).
    pub fn status(&self) -> i32 {
        self.status
    }

    /// The row this handle speaks for.
    pub fn row(&self) -> &'a BuiltinRow {
        self.row
    }

    /// Render one standard skeleton: `{prolog}{name}: {body}\n`.
    /// The getopt family (`InvalidOpt` / `NeedArg` / `NeedNumArg`)
    /// also sets `EX_USAGE`, matching the `.def` loops' `default:`
    /// arm (`builtin_usage(); return (EX_USAGE);`).
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
        let line = format!("{}{}: {body}\n", self.prolog.render(), self.row.name);
        self.sink.write_str(&line);
    }

    /// `builtin_usage` (`common.c:128-137`): `{name}: usage:
    /// {short_doc}` — with NO prolog. A row with no help topic
    /// (`env`) prints nothing: there is no short doc to print.
    pub fn usage(&mut self) {
        if let Some(short) = self.row.doc.short_doc() {
            let line = format!("{}: usage: {short}\n", self.row.name);
            self.sink.write_str(&line);
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
    pub fn os_error(&mut self, ctx: &str, err: &io::Error) {
        let text = crate::posix_errors::message(err);
        let line = format!("{}{}: {ctx}: {text}\n", self.prolog.render(), self.row.name);
        self.sink.write_str(&line);
    }

    /// A non-standard diagnostic body through the same prolog and
    /// sink: `{prolog}{name}: {message}\n`.
    pub fn free(&mut self, args: fmt::Arguments<'_>) {
        let line = format!("{}{}: {}\n", self.prolog.render(), self.row.name, args);
        self.sink.write_str(&line);
    }
}

/// `valid_number` (`general.c`): base-10 `strtoimax` over the
/// whole string modulo trailing whitespace; overflow invalid.
pub(crate) fn valid_number(s: &str) -> Option<i64> {
    let trimmed = s.trim_ascii();
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
        let mut diag = Diag::new(prolog_script(), cd_row(), &mut sink);
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
        let mut diag = Diag::new(
            prolog_script(),
            builtin_table_row("shopt").unwrap(),
            &mut sink,
        );
        diag.error(DiagKind::InvalidOpt("+z".to_string()));
        assert_eq!(
            String::from_utf8(sink).unwrap(),
            "t.sh: line 1: shopt: +z: invalid option\n"
        );
    }

    #[test]
    fn usage_has_no_prolog_and_env_prints_nothing() {
        let mut sink = Vec::new();
        let mut diag = Diag::new(prolog_script(), cd_row(), &mut sink);
        diag.usage();
        assert_eq!(
            String::from_utf8(sink).unwrap(),
            "cd: usage: cd [-L|[-P [-e]]] [-@] [dir]\n"
        );
        let mut sink = Vec::new();
        let mut diag = Diag::new(
            prolog_script(),
            builtin_table_row("env").unwrap(),
            &mut sink,
        );
        diag.usage();
        assert!(sink.is_empty(), "env has no short doc to print");
    }

    #[test]
    fn numeric_arg_ports_get_numeric_arg() {
        let w = |ss: &[&str]| ss.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let mut sink = Vec::new();
        let mut diag = Diag::new(
            prolog_script(),
            builtin_table_row("break").unwrap(),
            &mut sink,
        );
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
        let out = String::from_utf8(sink).unwrap();
        assert!(out.contains("break: 9999999999999999999999: numeric argument required\n"));
        assert!(out.contains("break: too many arguments\n"));
    }

    #[test]
    fn numeric_arg_soft_leaves_status_clear() {
        let w = |ss: &[&str]| ss.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let mut sink = Vec::new();
        let mut diag = Diag::new(prolog_script(), cd_row(), &mut sink);
        assert_eq!(diag.numeric_arg(&w(&["xx"]), NumericFatal::Soft), None);
        assert_eq!(diag.status(), 0, "fatal==0 sets no status");
    }

    #[test]
    fn check_identifier_uses_valid_identifier() {
        let mut sink = Vec::new();
        let mut diag = Diag::new(prolog_script(), cd_row(), &mut sink);
        assert!(diag.check_identifier("HOME"));
        assert!(diag.check_identifier("_x1"));
        assert!(!diag.check_identifier("1x"));
        assert!(!diag.check_identifier(""));
        assert!(!diag.check_identifier("a-b"));
        assert!(!diag.check_identifier("é"));
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
        let mut diag = Diag::new(prolog_script(), cd_row(), &mut sink);
        diag.os_error("/nonexistent", &io::Error::from_raw_os_error(2));
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
        let mut diag = Diag::new(prolog_interactive(), cd_row(), &mut sink);
        diag.free(format_args!("cannot cd to {}", "/tmp"));
        assert_eq!(
            String::from_utf8(sink).unwrap(),
            "bash: cd: cannot cd to /tmp\n"
        );
    }
}
