//! One session finish as an explicit, step-decomposed object
//! (unixwin/niubash#863 GAP-8).
//!
//! The engine has exactly one finish implementation (the
//! `finish_shell` pass), but until now its only contract face
//! was consuming: `Session::close` / `close_with_report` take
//! the session away and take their pending status from the
//! executor's last status. A host that finishes a session it
//! cannot consume — the interactive REPL's session lives in a
//! shared cell — or that must interleave its own duties INSIDE
//! the pass (a post-trap cwd record, a host-provider history
//! flush, the process exit itself) had no contract for any of
//! that, and composed the finish by hand through the tier-C
//! executor seam instead. This guard is the contract
//! replacement: the host opens one guard per finish from the
//! `&mut Session` it already holds, with the route's pending
//! status as a VALUE, and drives the finish's coarse steps
//! through the guard.
//!
//! Why a guard object, and not a single `finish(&mut self,
//! status)` face (the brief's option (a))? A single face can
//! serve the routes that interleave nothing, and
//! [`Finish::finish_all`] is exactly that call. But the REPL's
//! interleaving sits INSIDE the pass — trap, then the HIST*
//! refresh and the host provider's flush (which must observe
//! the session AS THE TRAP LEFT IT, niu#646), then exit — and
//! the REPL cannot consume its session at all. A monolith face
//! would leave that route hand-assembling around it, which is
//! the bridge pattern this GAP retires. So the honest shape
//! is the GAP-7 one: an explicit object owning the `&mut`
//! borrow for the finish, exposing the steps plus the outcome
//! as values.
//!
//! Order contract (captain adjudication D1, GNU-anchored):
//! on the plain path the EXIT trap runs BEFORE the engine
//! history write — GNU `exit_shell` (shell.c:985) runs the
//! trap at shell.c:1000-1002 and the history save at
//! shell.c:1007-1009, and `maybe_save_shell_history`
//! (bashhist.c:488) reads HISTFILE/HISTFILESIZE at the write,
//! so the trap's own edits and additions govern the save. On
//! a termsig death the sig.c:590-667 funnel order stands
//! exactly as built (history first, SIGHUP hangup before the
//! trap, die-by-signal last). The duties step makes that
//! choice once, internally, when it completes an un-run trap
//! step; a caller that runs the trap step standalone owns the
//! interleaving from that point.

use std::path::PathBuf;

use crate::script_driver::{FinishHistoryGate, FinishPass};

/// Whether the finish's stream-end duties write the ENGINE
/// session-history list to `$HISTFILE`.
///
/// The engine's session history and the interactive host's
/// provider history are different stores: a finish that wrote
/// both would double-write (the niu#642 failure family). The
/// consuming close defaults to `Run`; the interactive route,
/// whose history is the host provider's (flushed by the host
/// after the trap step), gates the step `Skip` — the duties
/// still flush job notifications, close persistent output
/// process substitutions, and run the hangup duties.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionHistoryWrite {
    /// Write the engine session-history list at the duties
    /// step, in the pass's canonical position.
    Run,
    /// Do not write the engine session-history list in this
    /// finish. The history step is consumed: a later
    /// `finish_all` on the same guard does not retro-write.
    Skip,
}

/// The per-route gates for [`Finish::run_stream_end_duties`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinishDuties {
    /// The engine session-history write gate for this route.
    pub history: SessionHistoryWrite,
}

impl Default for FinishDuties {
    /// The consuming close's defaults: every engine duty
    /// runs, the history write included.
    fn default() -> Self {
        Self {
            history: SessionHistoryWrite::Run,
        }
    }
}

/// What a completed finish reports (the finish boundary as
/// values, GAP-2's discipline: the host must not re-derive
/// close-boundary facts): the resolved status, the post-trap
/// working directory, and which steps actually ran.
/// `#[non_exhaustive]` so the reported set can grow within a
/// minor version, exactly as [`super::session::CloseReport`]'s can.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FinishOutcome {
    /// The final status: the pending status the guard was
    /// opened with, as the exit path resolved it (the trap's
    /// own `exit` overrides; plain completion keeps it).
    pub status: i32,
    /// The session's working directory at the end of the
    /// pass, read after the exit path ran — the value
    /// [`Finish::final_cwd`] reports once the trap step has
    /// run (the rubash#668 point).
    pub final_cwd: PathBuf,
    /// Whether the EXIT trap action actually fired during
    /// this finish — as opposed to being already consumed by
    /// an earlier funnel entry, cleared by a successful
    /// `exec`, or never registered.
    pub trap_ran: bool,
    /// Whether the duties step wrote the engine
    /// session-history list (false when the step was gated
    /// off, and when there was nothing to write).
    pub history_written: bool,
    /// Whether the stream-end duties step ran.
    pub duties_run: bool,
}

/// One session finish, borrowed from its session. Opened by
/// [`super::session::Session::begin_finish`]; see the module
/// documentation for the shape's rationale and the order
/// contract.
pub struct Finish<'a> {
    pass: FinishPass<'a>,
}

impl<'a> Finish<'a> {
    pub(crate) fn new(pass: FinishPass<'a>) -> Self {
        Self { pass }
    }

    /// The charter step: run the EXIT trap with the guard's
    /// pending status as its entry `$?`, and return the
    /// resolved status — the trap's own `exit` overrides,
    /// plain completion keeps the pending status (GNU
    /// trap.c:1027 `run_exit_trap`: the action's explicit
    /// `exit` makes the result, trap.c:1075-1076; plain
    /// completion keeps the saved pending value, trap.c:1081).
    /// The step carries the pass's pre-trap machinery with it
    /// (termsig marks, the SIGHUP hangup, the end-of-stream
    /// job-notification flush) in the order the built pass has
    /// always run them. Once-only: a later call returns the
    /// resolved status without re-running anything (the trap
    /// itself runs once per exit, rubash#2212).
    pub fn run_exit_trap(&mut self) -> i32 {
        self.pass.run_exit_trap()
    }

    /// The session's working directory at this point of the
    /// finish: read after [`Self::run_exit_trap`], this is the
    /// post-trap directory — the value that exists only
    /// between the trap and the session's consumption, which
    /// hosts used to park in a field to survive `close()`
    /// (rubash#668).
    pub fn final_cwd(&self) -> PathBuf {
        self.pass.final_cwd()
    }

    /// The stream-end duties as one once-only bundle: the
    /// engine session-history write (gated per route by
    /// `duties`), the debugger exit note, the persistent
    /// output process-substitution close, the huponexit
    /// hangup, and the termsig funnel's die-by-signal. If the
    /// trap step has not run yet, this step runs it — in the
    /// pass's canonical position for the order (see the
    /// module documentation): after the history write on a
    /// termsig death, before it on the plain path. Once-only:
    /// a later call is a no-op.
    pub fn run_stream_end_duties(&mut self, duties: FinishDuties) {
        let gate = match duties.history {
            SessionHistoryWrite::Run => FinishHistoryGate::Run,
            SessionHistoryWrite::Skip => FinishHistoryGate::Skip,
        };
        self.pass.run_stream_end_duties(gate);
    }

    /// The one-call finish for routes that interleave
    /// nothing: the trap step and the duties (history gate at
    /// its default, [`SessionHistoryWrite::Run`]) in the
    /// pass's canonical order, reported as a
    /// [`FinishOutcome`]. Safe to call after any of the steps
    /// ran standalone — every step is once-only, and the
    /// outcome reports what actually ran across the whole
    /// finish.
    pub fn finish_all(&mut self) -> FinishOutcome {
        self.run_stream_end_duties(FinishDuties::default());
        FinishOutcome {
            status: self.pass.resolved_status(),
            final_cwd: self.pass.final_cwd(),
            trap_ran: self.pass.trap_ran(),
            history_written: self.pass.history_written(),
            duties_run: self.pass.duties_done(),
        }
    }
}
