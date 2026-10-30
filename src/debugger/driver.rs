//! The session driver: the consult-side owner that pairs a
//! [`DebugSession`] with the stop loop that serves its stops
//! (debugger design sections 3/4; WP-3 consult wiring).
//!
//! The executor owns at most one driver — the session contract is
//! one thread, one session. While the debuggee runs, the executor
//! consults the driver through the DEBUG-trap funnel; when the
//! consult decides a stop, the driver serves it: the frontend's
//! stop loop runs (it sees only the session, never the executor),
//! and its outcome tells the executor how execution continues.
//!
//! Gate pairing lives with the driver layer, per the skeleton's
//! ruling: whoever installs a driver activates the session, and
//! termination or detach deactivates it. State transitions inside
//! [`DebugSession`] never touch the process gate.

use std::collections::HashSet;
use std::fmt;

use super::breakpoints::BreakpointId;
use super::evaluation::EvaluationRequest;
use super::session::{DebugSession, StepArming};

/// What the stop loop decided while the debuggee was parked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopLoopOutcome {
    /// Resume the debuggee under this arming.
    Resume(StepArming),
    /// Force the frame the debuggee is stopped in to return now
    /// (design 6.1, the `return [status]` command): the frame
    /// unwinds exactly as if it had executed `return`, carrying
    /// this status — or the frame's current `$?` when `None` —
    /// and the session stops in the caller at the return point
    /// under a finish-style arming. The consult applies this only
    /// inside a function or sourced-file frame; anywhere else it
    /// is rejected and the debuggee resumes under `Continue`.
    ForceReturn(Option<i32>),
    /// End the session's attachment: the gate is disarmed and the
    /// driver is dropped; the debuggee runs on unattached.
    Detach,
    /// Evaluate this request in the stopped frame (design
    /// 7.4/7.5; WP-7): the executor runs it through the
    /// evaluation machinery in `super::evaluation`, installs the
    /// outcome on the session as its last evaluation, refreshes
    /// the stop's inspection snapshot (an explicit evaluation may
    /// have written state), and serves the same stop again — the
    /// debuggee stays parked.
    Evaluate(EvaluationRequest),
}

/// The frontend's half of a stop: serve one parked session.
///
/// Implementations receive only the session — they may inspect
/// state, set or clear breakpoints, and drain events — and must
/// return how the debuggee continues. They never see the executor:
/// evaluation in the stopped frame is the consult's own machinery
/// (the section 5.3 condition path, WP-7's evaluation path), which
/// the stop loop reaches only by asking for it with
/// [`StopLoopOutcome::Evaluate`] and reading the outcome the
/// executor installs on the session.
pub trait StopLoop {
    /// Serve one stop. Called with the session already in the
    /// stopped state; return the outcome to apply.
    fn serve(&mut self, session: &mut DebugSession) -> StopLoopOutcome;

    /// Render a debugger-side announcement that is not a stop.
    /// The consult calls this while the debuggee is running —
    /// design 5.3's once-per-breakpoint condition failure report
    /// is the first user — so frontends with a mid-run channel
    /// render it immediately, at the consult that produced it.
    /// Frontends without one (and the test loops) inherit the
    /// no-op; the driver's record still stands.
    fn announce(&mut self, session: &DebugSession, text: &str) {
        let _ = (session, text);
    }
}

/// A session plus the stop loop that serves its stops.
pub struct SessionDriver {
    session: DebugSession,
    stop_loop: Box<dyn StopLoop>,
    /// The entry stop (DE-01) is an already-established stop: the
    /// session was constructed stopped, so the first funnel arrival
    /// serves it rather than arming anything new (WP-1 gap G-08;
    /// 150-c REPORT section 5, note 4). This flag is that pending
    /// serve; it is set at construction and consumed by the first
    /// funnel arrival.
    entry_stop_pending: bool,
    /// Breakpoints whose condition evaluation has already failed
    /// once: a failing condition is reported once per breakpoint
    /// (design 5.3), and this set is the record that it was.
    condition_error_reported: HashSet<BreakpointId>,
}

impl SessionDriver {
    /// Pair a fresh session with its stop loop. The session starts
    /// stopped at the entry stop, so the entry serve is pending.
    pub fn new(session: DebugSession, stop_loop: Box<dyn StopLoop>) -> Self {
        Self {
            session,
            stop_loop,
            entry_stop_pending: true,
            condition_error_reported: HashSet::new(),
        }
    }

    /// The session this driver serves.
    pub fn session(&self) -> &DebugSession {
        &self.session
    }

    /// The session, mutably (the consult and the stop loop both
    /// work through here).
    pub fn session_mut(&mut self) -> &mut DebugSession {
        &mut self.session
    }

    /// Whether the entry stop still waits for its first serve.
    pub fn entry_stop_pending(&self) -> bool {
        self.entry_stop_pending
    }

    /// Consume the entry-stop serve: true exactly once, at the
    /// first funnel arrival after installation.
    pub fn take_entry_stop_pending(&mut self) -> bool {
        std::mem::take(&mut self.entry_stop_pending)
    }

    /// Run the stop loop against the parked session.
    pub fn serve_stop(&mut self) -> StopLoopOutcome {
        self.stop_loop.serve(&mut self.session)
    }

    /// Record a condition evaluation failure for `id`; true when
    /// this is the first failure for that breakpoint (and so the
    /// one the frontend reports).
    pub fn note_condition_error(&mut self, id: BreakpointId) -> bool {
        self.condition_error_reported.insert(id)
    }

    /// Design 5.3's report outlet: record a condition evaluation
    /// failure for `id` and, when it is the breakpoint's first,
    /// announce the rendered report through the stop loop. Later
    /// failures of the same breakpoint stay silent (the failure
    /// is reported once per breakpoint, not per hit).
    pub fn announce_condition_error(&mut self, id: BreakpointId, report: &str) {
        if self.note_condition_error(id) {
            self.stop_loop.announce(&self.session, report);
        }
    }
}

/// Teardown disarms the gate (PLAN row 11, the row-5 tail).
///
/// Gate pairing has exactly three legitimate ends — termination
/// (`debugger_note_exit`), detach (the serve loop takes the
/// driver out and deactivates), and this one: the driver is
/// dropped without either, because its owner went away mid-run
/// (an executor dropped with a session still installed, an
/// install replaced by a newer driver, a frontend returning
/// early). Before this impl, the third end leaked: the process
/// gate stayed armed with no session behind it, and every later
/// seam paid a consult against a dead driver — the engine never
/// returned to the inert state (S4 state (1)).
///
/// Disarming is an idempotent atomic store, so dropping a driver
/// whose session already terminated or detached is a no-op; and
/// installation activates *after* storing the new driver
/// (`install_debug_session`), so a replaced driver's drop —
/// which happens during the store — lands before the new
/// session's activation, never after it. The single-session
/// contract (one thread, one session) is what makes a bare
/// disarm sound: there is never a second live session whose
/// gate this could lower.
impl Drop for SessionDriver {
    fn drop(&mut self) {
        self.session.deactivate();
    }
}

impl fmt::Debug for SessionDriver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionDriver")
            .field("session", &self.session)
            .field("entry_stop_pending", &self.entry_stop_pending)
            .field("condition_error_reported", &self.condition_error_reported)
            .finish_non_exhaustive()
    }
}
