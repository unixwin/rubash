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
use super::session::{DebugSession, StepArming};

/// What the stop loop decided while the debuggee was parked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopLoopOutcome {
    /// Resume the debuggee under this arming.
    Resume(StepArming),
    /// End the session's attachment: the gate is disarmed and the
    /// driver is dropped; the debuggee runs on unattached.
    Detach,
}

/// The frontend's half of a stop: serve one parked session.
///
/// Implementations receive only the session — they may inspect
/// state, set or clear breakpoints, and drain events — and must
/// return how the debuggee continues. They never see the executor:
/// evaluation in the stopped frame is the consult's own machinery
/// (the section 5.3 condition path), not the stop loop's.
pub trait StopLoop {
    /// Serve one stop. Called with the session already in the
    /// stopped state; return the outcome to apply.
    fn serve(&mut self, session: &mut DebugSession) -> StopLoopOutcome;
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
