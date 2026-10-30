//! The event observer (API design §5.6): one trait, default
//! no-op methods, registered on the builder. Events are
//! notifications, not control flow — an observer cannot veto a
//! command. (The two historical exceptions stay where they
//! already live as capabilities, not events: history recording
//! veto inside the interactive line path, and rc policy.)
//!
//! Production boundary of this publication step, per method:
//! `before_command` / `after_command` fire once per
//! [`Session::run`](super::Session) submission — the
//! per-statement funnel stops are the debugger's consult
//! surface, not this observer's granularity.
//! `cwd_changed` and `exported_variable_changed` are diffed at
//! run boundaries (snapshots compared around the run, only when
//! an observer is registered). `prompt_ready` fires from
//! [`Session::expand_prompt`](super::Session::expand_prompt).
//! `exit_requested` fires when a run's flow is
//! [`Flow::ExitRequested`](super::Flow). `job_status_changed`
//! has **no producer in this step**: job notices still travel
//! the engine's text path at stream end (inside the close
//! path's finish machinery); the method exists because the
//! trait is the growth surface and its default keeps it free.

use std::path::Path;
use std::time::Duration;

/// One submitted run, as the observer sees it: the source text
/// verbatim, its document identity, and the line offset it
/// carried.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CommandEvent {
    pub text: String,
    pub source_name: Option<String>,
    pub line_offset: usize,
}

/// A background job's identity and terminal status, for
/// [`SessionEvents::job_status_changed`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct JobInfo {
    pub id: i32,
    pub status: i32,
    pub command: String,
}

/// The session event observer (§5.6). All methods default to
/// no-ops; implement only the ones a host renders from.
pub trait SessionEvents {
    /// A run submission is about to execute.
    fn before_command(&mut self, _event: &CommandEvent) {}

    /// A run submission completed, with its final status and
    /// wall-clock duration.
    fn after_command(&mut self, _event: &CommandEvent, _status: i32, _elapsed: Duration) {}

    /// The session's working directory changed across a run.
    fn cwd_changed(&mut self, _new: &Path) {}

    /// An exported variable was added or changed (`Some`) or
    /// removed (`None`) across a run.
    fn exported_variable_changed(&mut self, _name: &str, _value: Option<&str>) {}

    /// A prompt string was produced by
    /// [`Session::expand_prompt`](super::Session::expand_prompt)
    /// — prompts travel this channel, never the output stream.
    fn prompt_ready(&mut self, _prompt: &str) {}

    /// A run asked the session to exit with this status.
    fn exit_requested(&mut self, _status: i32) {}

    /// A background job changed state. No producer in this
    /// publication step (see the module boundary note).
    fn job_status_changed(&mut self, _job: &JobInfo) {}
}
