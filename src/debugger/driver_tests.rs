//! SessionDriver teardown pins (PLAN row 11; the row-5 tail).
//!
//! The gate pairing's third end: a driver dropped without
//! termination or detach must still disarm the process gate, so
//! the engine returns to the inert state (S4 state (1)) — one
//! atomic load, no consult. These tests pin the Drop disarm, the
//! install ordering that makes replacement safe, and the
//! observable consequence: after teardown, a fresh run behaves
//! exactly as an un-debugged run.

use super::driver::{SessionDriver, StopLoop, StopLoopOutcome};
use super::gate::SESSION_ACTIVE;
use super::session::{DebugSession, StepArming};
use crate::executor::Executor;

/// A stop loop that resumes under Continue and counts its serves.
struct ContinueLoop;

impl StopLoop for ContinueLoop {
    fn serve(&mut self, _session: &mut DebugSession) -> StopLoopOutcome {
        StopLoopOutcome::Resume(StepArming::Continue)
    }
}

fn driver() -> SessionDriver {
    SessionDriver::new(DebugSession::new(), Box::new(ContinueLoop))
}

/// Holds the gate test lock for the whole armed span and disarms
/// on exit, panic paths included (the wiring_tests pattern).
struct GateTestSpan {
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl GateTestSpan {
    fn new() -> Self {
        let lock = super::gate::GATE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Self { _lock: lock }
    }
}

impl Drop for GateTestSpan {
    fn drop(&mut self) {
        SESSION_ACTIVE.disarm();
    }
}

#[test]
fn dropping_a_driver_without_termination_disarms_the_gate() {
    let _span = GateTestSpan::new();
    let driver = driver();
    driver.session().activate();
    assert!(SESSION_ACTIVE.is_armed());
    drop(driver);
    assert!(
        !SESSION_ACTIVE.is_armed(),
        "teardown returns the engine to the inert state"
    );
}

#[test]
fn dropping_an_executor_with_a_session_installed_disarms_the_gate() {
    let _span = GateTestSpan::new();
    let mut executor = Executor::new();
    executor.install_debug_session(driver());
    assert!(SESSION_ACTIVE.is_armed());
    drop(executor);
    assert!(
        !SESSION_ACTIVE.is_armed(),
        "an abandoned session must not leave the gate armed"
    );

    // The observable consequence: a fresh executor runs with no
    // session and no stops — the un-debugged behavior, byte for
    // byte the inert state.
    let mut fresh = Executor::new();
    assert!(fresh.debug_driver().is_none());
    let status = crate::script_driver::run_source(&mut fresh, "true\n", false);
    assert_eq!(status, 0);
    assert!(!SESSION_ACTIVE.is_armed());
}

#[test]
fn installing_over_an_installed_driver_keeps_the_new_session_armed() {
    let _span = GateTestSpan::new();
    let mut executor = Executor::new();
    executor.install_debug_session(driver());
    // The replacement carries a marker breakpoint so the test can
    // tell which session survived.
    let mut replacement = driver();
    replacement
        .session_mut()
        .set_line_breakpoint("marker.sh", 7);
    executor.install_debug_session(replacement);
    assert!(
        SESSION_ACTIVE.is_armed(),
        "the replaced driver's Drop disarm lands before the new activation"
    );
    let installed = executor.debug_driver().expect("a driver is installed");
    assert_eq!(
        installed.session().breakpoints().len(),
        1,
        "the installed session is the replacement, marker and all"
    );
}
