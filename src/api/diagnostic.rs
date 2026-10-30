//! Diagnostics as values (API design §6.3) and the boundary's
//! error types (Q7, decided 2026-10-25: new `#[non_exhaustive]`
//! enums wrapping internal errors opaquely — the engine's
//! `ExecuteError` is not re-exported as tier A).

use crate::executor::ExecuteError;

/// The class of a [`Diagnostic`] (§6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DiagnosticKind {
    Parse,
    Runtime,
    Builtin,
    Expansion,
}

/// One diagnostic as data (§6.3): the GNU-worded message, the
/// source identity and line when the engine knows them, and the
/// exit status the diagnostic carried, if any.
///
/// The same diagnostic also reaches the session's error stream in
/// text form with unchanged wording (the goldens pin the text);
/// this value form exists so a host can render, index, or route a
/// diagnostic without parsing text.
///
/// Production boundary of this publication step: value-form
/// diagnostics are produced by [`Session::parse`](super::Session)
/// — extracted from the parser's own error nodes — and delivered
/// to the session's [`DiagnosticSink`](super::DiagnosticSink).
/// Diagnostics raised while a run executes are still produced
/// inside the drivers as text, so
/// [`RunOutcome::diagnostics`](super::RunOutcome) is empty for
/// `run()` in this step: a value channel for runtime diagnostics
/// needs engine instrumentation and is recorded as future work in
/// `docs/api-ledger.md`, not silently faked here.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Diagnostic {
    pub kind: DiagnosticKind,
    /// The GNU-worded message. For the parser's plain error
    /// marker this is the stored message verbatim — the same text
    /// the text form prints after its prefix (the token-class
    /// framing, ``syntax error near …``, is applied by the text
    /// form's renderer; the value carries the parser's message).
    pub message: String,
    pub source_name: Option<String>,
    /// 1-based line within the source the diagnostic belongs to.
    pub line: Option<usize>,
    /// The exit status the diagnostic carried, if any.
    pub status: Option<i32>,
}

/// A parse that did not produce a unit (Q7's `ParseFailure`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseFailure {
    /// The source does not parse; the diagnostic is the first
    /// syntax error in the shape §6.3 defines.
    Syntax(Diagnostic),
}

impl ParseFailure {
    /// The diagnostic this failure carries.
    pub fn diagnostic(&self) -> &Diagnostic {
        match self {
            ParseFailure::Syntax(diagnostic) => diagnostic,
        }
    }
}

impl std::fmt::Display for ParseFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseFailure::Syntax(diagnostic) => write!(f, "{}", diagnostic.message),
        }
    }
}

impl std::error::Error for ParseFailure {}

/// The session boundary's error type (Q7).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SessionError {
    /// Session construction failed: a builder input the engine
    /// refused (for example a working directory the `cd`
    /// machinery cannot enter). No session exists.
    Build { message: String },
    /// A driven operation failed in the engine. `status` is the
    /// shell status the failure carries when it has one (an
    /// exit-class failure carries its code, a command-not-found
    /// carries 127, a signal unwind carries 128 + signal); the
    /// message is the engine's own wording.
    Execution {
        status: Option<i32>,
        message: String,
    },
}

impl SessionError {
    /// The shell status this error carries, if any.
    pub fn status(&self) -> Option<i32> {
        match self {
            SessionError::Build { .. } => None,
            SessionError::Execution { status, .. } => *status,
        }
    }

    pub(crate) fn build(message: impl Into<String>) -> SessionError {
        SessionError::Build {
            message: message.into(),
        }
    }

    /// Wrap an engine execution failure opaquely (Q7): the
    /// boundary exposes the status the failure carries and the
    /// engine's wording, not the internal variant.
    pub(crate) fn from_execute_error(error: &ExecuteError) -> SessionError {
        let status = match error {
            ExecuteError::ExitCode(status)
            | ExecuteError::Return(status)
            | ExecuteError::ExpansionFailure(status)
            | ExecuteError::FatalFunctionError(status)
            | ExecuteError::LastpipeExit(status) => Some(*status),
            ExecuteError::CommandNotFound(_) => Some(127),
            ExecuteError::SigintUnwind => Some(130),
            _ => None,
        };
        SessionError::Execution {
            status,
            message: error.to_string(),
        }
    }
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionError::Build { message } => {
                write!(f, "session construction failed: {message}")
            }
            SessionError::Execution { message, .. } => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for SessionError {}
