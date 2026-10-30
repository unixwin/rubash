//! The session gate flag (debugger design section 4.4; WP-1 gap
//! register G-02).
//!
//! One atomic flag answers "is a debug session attached?" for the
//! whole process. Seam admission consults it before any heavier
//! session work — the design places the load ahead of the trap-action
//! presence check — so a run with no session pays one atomic load and
//! one branch. That is the debugger-off zero-cost contract; it holds
//! here by construction, because nothing else in this module runs
//! unless the flag is armed.
//!
//! The flag does not identify *which* session is attached. The
//! session contract is one thread owning one session, so there is at
//! most one live session to consult, and the consult itself happens
//! on the owning thread.

use std::sync::atomic::{AtomicBool, Ordering};

/// An arm/disarm flag with the session gate's semantics.
///
/// Kept as a type (rather than a bare static) so tests and future
/// per-context gates can hold their own instances; the process-wide
/// gate is [`SESSION_ACTIVE`].
#[derive(Debug)]
pub struct GateFlag {
    flag: AtomicBool,
}

impl GateFlag {
    /// A disarmed flag: no session attached.
    pub const fn new() -> Self {
        Self {
            flag: AtomicBool::new(false),
        }
    }

    /// Mark a session as attached. Called by session activation,
    /// before the debuggee's first command can reach a seam.
    pub fn arm(&self) {
        self.flag.store(true, Ordering::Release);
    }

    /// Mark the session as detached (terminated or never started).
    pub fn disarm(&self) {
        self.flag.store(false, Ordering::Release);
    }

    /// The seam-admission question: is a session attached?
    pub fn is_armed(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }
}

impl Default for GateFlag {
    fn default() -> Self {
        Self::new()
    }
}

/// The one session gate for the process.
///
/// Armed exactly while a [`super::DebugSession`] is active. When it
/// is disarmed, seam code must not consult anything else in this
/// module — that is what makes the debugger-off path cost a single
/// atomic load.
pub static SESSION_ACTIVE: GateFlag = GateFlag::new();

/// Serializes tests that arm the process-wide gate: the gate is
/// global state in a multi-threaded test binary, so every test
/// that arms it (the roundtrip below, the WP-3 wiring tests) holds
/// this lock for the whole armed span.
#[cfg(test)]
pub(crate) static GATE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::{GateFlag, SESSION_ACTIVE};

    #[test]
    fn new_flag_starts_disarmed() {
        let flag = GateFlag::new();
        assert!(!flag.is_armed());
    }

    #[test]
    fn default_flag_starts_disarmed() {
        let flag = GateFlag::default();
        assert!(!flag.is_armed());
    }

    #[test]
    fn arm_disarm_roundtrip() {
        let flag = GateFlag::new();
        flag.arm();
        assert!(flag.is_armed());
        flag.arm();
        assert!(flag.is_armed(), "arming twice is a no-op state");
        flag.disarm();
        assert!(!flag.is_armed());
        flag.disarm();
        assert!(!flag.is_armed(), "disarming twice is a no-op state");
    }

    /// The process-wide gate is touched by exactly this one test, so
    /// its arm/disarm sequence cannot race another test in this
    /// binary: session tests never activate a session against the
    /// global flag (activation is an explicit, separate step from
    /// construction and state transitions).
    #[test]
    fn session_active_global_roundtrip() {
        // Acquire the lock the way every GateTestSpan does —
        // poison-tolerantly. A span holder that panics mid-span
        // poisons this mutex; the flag itself is restored by the
        // span's Drop disarm, so this test recovers the guard
        // instead of exploding on the poison.
        let _guard = crate::debugger::gate::GATE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(!SESSION_ACTIVE.is_armed());
        SESSION_ACTIVE.arm();
        assert!(SESSION_ACTIVE.is_armed());
        SESSION_ACTIVE.disarm();
        assert!(!SESSION_ACTIVE.is_armed());
    }
}
