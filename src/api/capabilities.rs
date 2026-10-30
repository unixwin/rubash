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
//! - [`ExternalRouteProvider`] — the route seam (niubash#567):
//!   run a RESOLVED foreground external in-process over live OS
//!   stdio handles with the complete child environment, the
//!   session cwd, and a cancellation poll, or decline to the
//!   prepared spawn. Unlike the buffered handler above, it is
//!   consulted after resolution and redirection, with the spawn
//!   already prepared — the seam an FFI provider binds to.
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
pub use crate::executor::{
    ExternalRouteHandle, ExternalRouteOutcome, ExternalRouteProvider, ExternalRouteRequest,
    ExternalRouteStdio,
};

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
