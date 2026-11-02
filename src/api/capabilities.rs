//! Host services: the capability traits of API design §5.5,
//! injected at build time on [`SessionBuilder`](super::SessionBuilder).
//! Each generalizes a mechanism that exists today; capability
//! traits return data or decisions, and they are the only places
//! a host can alter execution.
//!
//! - [`HistoryProvider`](super::HistoryProvider) — the existing
//!   trait, re-exported unchanged (the design's instruction for
//!   it is "keep"); it is not redefined here.
//! - [`ExternalCommandHandler`] — formalizes the engine's
//!   host-external-command closure setter: decide/run-or-delegate
//!   for non-builtin commands, with the session's argv and
//!   environment as data.
//! - [`ElevationHandler`] (Windows) — formalizes the elevation
//!   closure setter: sudo/elevation requests as data.
//! - [`CompletionProvider`] — backing for
//!   [`Session::complete`](super::Session::complete): commands
//!   only the host knows. In this step the session consults it
//!   when completing (its candidates merge after the engine's),
//!   which keeps the engine's own programmable-completion
//!   semantics untouched.
//! - [`RcPolicy`] — the host decides profile/rc sourcing per
//!   session (§6.6): the engine stops assuming it owns startup.
//!   Without an injected policy, the session sources the
//!   engine's own invocation lists — the same lists the shipped
//!   binary's `run_startup_files` uses.
//! - [`DiagnosticSink`] — receives [`Diagnostic`] values (§6.3)
//!   as they are produced (in this step: from
//!   [`Session::parse`](super::Session::parse)).

use std::collections::HashMap;
use std::path::PathBuf;

use super::Diagnostic;
use crate::executor::HostExternalCommandOutput;
#[cfg(windows)]
use crate::executor::{ElevationOutput, ElevationRequest};
/// Decide/run-or-delegate for a non-builtin command (§5.5).
/// Returning `None` delegates to the engine's normal spawn path;
/// returning `Some` supplies the command's whole result.
pub trait ExternalCommandHandler {
    fn handle_command(
        &mut self,
        argv: &[String],
        env: &HashMap<String, String>,
    ) -> Option<HostExternalCommandOutput>;
}

/// Sudo/elevation requests as data (Windows; §5.5).
#[cfg(windows)]
pub trait ElevationHandler {
    fn elevate(&mut self, request: ElevationRequest) -> Result<ElevationOutput, String>;
}

/// One completion candidate (§6.2): the text to insert and an
/// optional description the host's renderer may show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub text: String,
    pub description: Option<String>,
}

impl Candidate {
    /// A candidate without a description.
    pub fn new(text: impl Into<String>) -> Candidate {
        Candidate {
            text: text.into(),
            description: None,
        }
    }

    /// A candidate with a description.
    pub fn described(text: impl Into<String>, description: impl Into<String>) -> Candidate {
        Candidate {
            text: text.into(),
            description: Some(description.into()),
        }
    }
}

/// Host commands the engine cannot know (§5.5): consulted by
/// [`Session::complete`](super::Session::complete) for the same
/// line and cursor; the returned candidates merge after the
/// engine's own (a text the engine already produced is not
/// duplicated).
pub trait CompletionProvider {
    fn candidates(&mut self, line: &str, cursor: usize) -> Vec<Candidate>;
}

/// How the host editor should treat the engine's candidates for one
/// completion attempt — the port of pcomplete.c:1458-1477
/// `pcomp_set_readline_variables`, which bashline.c calls with the
/// USED compspec's options (compopt mutations included) to set the
/// `rl_*` variables the host's display/insertion code consumes.
/// Before rubash#2573 the `-o nospace` / `-o noquote` /
/// `-o filenames` / `-o fullquote` / `-o nosort` options were parsed
/// and printed but never crossed this channel, so no host could
/// honor them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompletionPolicy {
    /// COPT_FILENAMES → `rl_filename_completion_desired`: the
    /// candidates are filenames; the editor applies filename
    /// quoting/display treatment.
    pub filename_completion_desired: bool,
    /// COPT_FULLQUOTE → `rl_full_quoting_desired`: quote the inserted
    /// match fully, even when it contains no special characters.
    pub full_quoting_desired: bool,
    /// COPT_NOSPACE → `rl_completion_suppress_append`: insert the
    /// match WITHOUT appending a space (the input to the host's
    /// append-whitespace decision).
    pub suppress_append: bool,
    /// NOT COPT_NOQUOTE → `rl_filename_quoting_desired`: filenames
    /// among the candidates may be quoted on insertion. True unless
    /// the governing compspec carries `-o noquote`.
    pub filename_quoting_desired: bool,
    /// NOT COPT_NOSORT → `rl_sort_completion_matches`: the host may
    /// sort the matches for display; with `-o nosort` the engine
    /// preserves generation order and the host must not re-sort.
    /// True unless the governing compspec carries `-o nosort`.
    pub sort_matches: bool,
}

impl Default for CompletionPolicy {
    fn default() -> Self {
        CompletionPolicy {
            filename_completion_desired: false,
            full_quoting_desired: false,
            suppress_append: false,
            filename_quoting_desired: true,
            sort_matches: true,
        }
    }
}

/// The result of one engine completion attempt: the candidates plus
/// the insertion/display policy of the compspec that governed the
/// attempt (see [`super::Session::complete_detailed`]).
#[derive(Debug, Clone, Default)]
pub struct CompletionOutcome {
    pub candidates: Vec<Candidate>,
    pub policy: CompletionPolicy,
}

/// Receives [`Diagnostic`] values as they are produced (§6.3).
pub trait DiagnosticSink {
    fn diagnostic(&mut self, diagnostic: &Diagnostic);
}

/// Which startup-file class a sourcing decision is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StartupClass {
    /// The login profile chain (`/etc/profile`, then the first
    /// existing personal profile).
    LoginProfile,
    /// The non-login interactive rc chain.
    InteractiveRc,
    /// The `$BASH_ENV` file of a non-interactive shell.
    EnvFile,
}

/// What an [`RcPolicy`] decides from: the session facts the
/// engine's own list functions consult, precomputed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct StartupContext {
    /// The session's `$HOME`, when set.
    pub home: Option<String>,
    /// The `--rcfile` replacement for the personal-rc slot, when
    /// the host supplied one.
    pub rc_override: Option<String>,
    /// The expanded `$BASH_ENV` file, when it resolves to an
    /// existing regular file.
    pub bash_env: Option<String>,
}

/// The host decides startup sourcing per session (§5.5, §6.6).
/// The returned files are sourced in order with the engine's
/// startup-file semantics (a parse error aborts only that file;
/// an `exit` in one ends the chain and is reported through the
/// run outcome's flow).
pub trait RcPolicy {
    fn startup_files(&mut self, class: StartupClass, context: &StartupContext) -> Vec<PathBuf>;
}
