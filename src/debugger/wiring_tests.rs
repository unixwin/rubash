//! WP-3 wiring acceptance tests: the session core, consulted
//! through the DEBUG-trap funnel, stops a real debuggee.
//!
//! These tests drive an [`Executor`] over a sourced script file
//! with a scripted stop loop and assert the stop sequence, the
//! event stream, and the debuggee's observable state (variables
//! and final status). Test names carry the design entry they pin
//! (DE-xx catalogue ids, WP-1 gap ids, and the P-1 adjudication).
//!
//! Every test that arms the process gate holds
//! [`super::gate::GATE_TEST_LOCK`] for its whole span: the gate is
//! process-global and the lib test binary is multi-threaded.

use std::cell::RefCell;
use std::rc::Rc;

use super::driver::{SessionDriver, StopLoop, StopLoopOutcome};
use super::event::{DebugEvent, SeamKind, StopReason};
use super::session::{DebugSession, StepArming};
use crate::executor::Executor;

/// One served stop, recorded as the frontend saw it.
#[derive(Debug, Clone, PartialEq)]
struct StopRecord {
    reason: StopReason,
    file: Option<String>,
    line: Option<u32>,
    seam: Option<SeamKind>,
    frame_depth: usize,
    command_text: Option<String>,
}

type Handler = Box<dyn FnMut(&mut DebugSession, &StopRecord) -> StopLoopOutcome>;

struct RecordingLoop {
    stops: Rc<RefCell<Vec<StopRecord>>>,
    events: Rc<RefCell<Vec<DebugEvent>>>,
    handler: Handler,
}

impl StopLoop for RecordingLoop {
    fn serve(&mut self, session: &mut DebugSession) -> StopLoopOutcome {
        let info = session
            .last_stop()
            .cloned()
            .expect("a stop-loop serve implies a stop");
        let record = StopRecord {
            reason: info.reason,
            file: info.location.as_ref().map(|loc| loc.file.clone()),
            line: info.location.as_ref().map(|loc| loc.line),
            seam: info.seam,
            frame_depth: info.frame_depth,
            command_text: info.command_text.clone(),
        };
        self.stops.borrow_mut().push(record.clone());
        self.events.borrow_mut().extend(session.drain_events());
        (self.handler)(session, &record)
    }
}

/// Holds the gate test lock and guarantees the gate is disarmed
/// when the test ends, even on a panic path.
struct GateTestSpan {
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl GateTestSpan {
    fn new() -> Self {
        let lock = crate::debugger::gate::GATE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Self { _lock: lock }
    }
}

impl Drop for GateTestSpan {
    fn drop(&mut self) {
        crate::debugger::SESSION_ACTIVE.disarm();
    }
}

/// An executor with a session installed, plus the stop/event
/// records its stop loop collected.
struct Wired {
    executor: Executor,
    stops: Rc<RefCell<Vec<StopRecord>>>,
    events: Rc<RefCell<Vec<DebugEvent>>>,
}

fn wired_executor(handler: Handler) -> Wired {
    let stops = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let driver = SessionDriver::new(
        DebugSession::new(),
        Box::new(RecordingLoop {
            stops: stops.clone(),
            events: events.clone(),
            handler,
        }),
    );
    let mut executor = Executor::new();
    executor.install_debug_session(driver);
    Wired {
        executor,
        stops,
        events,
    }
}

/// Write `script` to a fresh temp file and return the driver input
/// that sources it plus the file's path string.
///
/// The path reaches the driver input **quoted**. It is a
/// native-spelling path, and in a bare word the lexer treats `\`
/// as an escape character: on Windows an unquoted
/// `C:\Users\...` loses every separator, `.` cannot find the
/// file, and the debuggee never runs (status 1, zero stops) —
/// the harness shape the api consumer tests hit before this
/// helper's callers did. Quoting delivers the path verbatim;
/// this helper is the only interpolation point for every test
/// in this module, so the quote lives here, not per test.
fn sourced_script(name: &str, script: &str) -> (String, String) {
    let dir = std::env::temp_dir().join(format!("rubash-150-wiring-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(format!("{name}.sh"));
    std::fs::write(&path, script).expect("write script");
    let path = path.to_string_lossy().into_owned();
    (format!(". \"{path}\""), path)
}

fn env_of(executor: &Executor, name: &str) -> Option<String> {
    executor.get_env(name).map(|value| value.to_string())
}

/// DE-01 / G-08: the entry stop is served at the first funnel
/// arrival — before the debuggee's first command executes — so a
/// breakpoint the frontend sets while parked there applies from
/// the very first line.
#[test]
fn de01_entry_stop_served_before_first_command() {
    let _span = GateTestSpan::new();
    let (input, path) = sourced_script("entry", "A=1\nB=2\nB=3\n");

    // Phase 1: step from the entry stop to learn the file identity
    // the engine reports for the sourced file.
    let mut discovered: Option<String> = None;
    {
        let mut wired = wired_executor(Box::new(|_session, _record| {
            StopLoopOutcome::Resume(StepArming::StepInto)
        }));
        let status = crate::script_driver::run_source(&mut wired.executor, &input, false);
        assert_eq!(status, 0);
        let stops = wired.stops.borrow();
        assert_eq!(
            stops[0].reason,
            StopReason::Entry,
            "first stop is the entry stop"
        );
        let step = stops
            .iter()
            .find(|stop| {
                stop.reason == StopReason::Step && stop.file.as_deref() == Some(path.as_str())
            })
            .expect("stepping from the entry stop stops inside the script");
        discovered = step.file.clone();
        assert_eq!(
            step.line,
            Some(1),
            "the first in-file step lands on its first line"
        );
    }
    let file = discovered.expect("file identity discovered");
    assert_eq!(file, path, "the engine reports the sourced path verbatim");

    // Phase 2: with the identity known, a line breakpoint set at
    // the entry stop fires on line 1 — proof the entry serve
    // preceded the first command's execution.
    let file_for_bp = file.clone();
    let mut wired = wired_executor(Box::new(move |session, record| {
        if record.reason == StopReason::Entry {
            session
                .breakpoints_mut()
                .insert_line(file_for_bp.clone(), 1);
        }
        StopLoopOutcome::Resume(StepArming::Continue)
    }));
    let status = crate::script_driver::run_source(&mut wired.executor, &input, false);
    assert_eq!(status, 0);
    {
        let stops = wired.stops.borrow();
        assert_eq!(
            stops.len(),
            2,
            "entry stop, then the line-1 breakpoint stop"
        );
        assert_eq!(stops[0].reason, StopReason::Entry);
        assert!(matches!(stops[1].reason, StopReason::Breakpoint(_)));
        assert_eq!(stops[1].line, Some(1));
        assert_eq!(stops[1].command_text.as_deref(), Some("A=1"));
    }
    assert_eq!(env_of(&wired.executor, "A").as_deref(), Some("1"));
    assert_eq!(env_of(&wired.executor, "B").as_deref(), Some("3"));
}

/// DE-02: a line breakpoint parks the debuggee mid-script; the
/// commands after the stop run only once the stop loop resumes.
/// With no session attached the same script runs identically
/// (the debugger-off contract).
#[test]
fn de02_line_breakpoint_parks_and_resume_completes() {
    let _span = GateTestSpan::new();
    let (input, path) = sourced_script("linebp", "A=1\nB=2\nB=3\n");

    let file_for_bp = path.clone();
    let mut wired = wired_executor(Box::new(move |session, record| {
        if record.reason == StopReason::Entry {
            session
                .breakpoints_mut()
                .insert_line(file_for_bp.clone(), 3);
        }
        StopLoopOutcome::Resume(StepArming::Continue)
    }));
    let status = crate::script_driver::run_source(&mut wired.executor, &input, false);
    assert_eq!(status, 0);
    {
        let stops = wired.stops.borrow();
        assert_eq!(stops.len(), 2);
        assert!(matches!(stops[1].reason, StopReason::Breakpoint(_)));
        assert_eq!(stops[1].line, Some(3));
        assert_eq!(stops[1].seam, Some(SeamKind::SimpleCommand));
        assert_eq!(stops[1].command_text.as_deref(), Some("B=3"));
    }
    assert_eq!(env_of(&wired.executor, "B").as_deref(), Some("3"));

    // Debugger-off: identical script, identical outcome.
    let mut bare = Executor::new();
    let bare_status = crate::script_driver::run_source(&mut bare, &input, false);
    assert_eq!(bare_status, status);
    assert_eq!(env_of(&bare, "B").as_deref(), Some("3"));
}

/// DE-03: a function breakpoint stops at the function-entry seam,
/// inside the callee's frame, before the body runs.
#[test]
fn de03_function_breakpoint_stops_at_entry_seam() {
    let _span = GateTestSpan::new();
    let (input, _path) = sourced_script("fnbp", "f() { G=inside; }\nf\nDONE=1\n");

    let mut wired = wired_executor(Box::new(move |session, record| {
        if record.reason == StopReason::Entry {
            session.breakpoints_mut().insert_function("f");
        }
        StopLoopOutcome::Resume(StepArming::Continue)
    }));
    let status = crate::script_driver::run_source(&mut wired.executor, &input, false);
    assert_eq!(status, 0);
    {
        let stops = wired.stops.borrow();
        assert_eq!(
            stops.len(),
            2,
            "entry stop, then the function breakpoint stop"
        );
        assert!(matches!(stops[1].reason, StopReason::FunctionBreakpoint(_)));
        assert_eq!(stops[1].seam, Some(SeamKind::FunctionEntry));
        assert_eq!(
            stops[1].frame_depth, 1,
            "the stop is inside the callee's frame"
        );
    }
    assert_eq!(env_of(&wired.executor, "G").as_deref(), Some("inside"));
    assert_eq!(env_of(&wired.executor, "DONE").as_deref(), Some("1"));
}

/// DE-04: a completed frame return is reported to the session,
/// and a `Finish` arming installed at the entry stop's successor
/// stops in the caller when that frame returns.
#[test]
fn de04_frame_return_event_and_finish_arming() {
    let _span = GateTestSpan::new();
    let (input, _path) = sourced_script("finish", "f() { G=inside; }\nf\nDONE=1\n");

    let mut wired = wired_executor(Box::new(move |session, record| match record.reason {
        StopReason::Entry => {
            session.breakpoints_mut().insert_function("f");
            StopLoopOutcome::Resume(StepArming::Continue)
        }
        StopReason::FunctionBreakpoint(_) => StopLoopOutcome::Resume(StepArming::Finish {
            frame_depth: record.frame_depth,
        }),
        _ => StopLoopOutcome::Resume(StepArming::Continue),
    }));
    let status = crate::script_driver::run_source(&mut wired.executor, &input, false);
    assert_eq!(status, 0);
    {
        let stops = wired.stops.borrow();
        assert_eq!(stops.len(), 3, "entry, function breakpoint, finish stop");
        assert_eq!(stops[2].reason, StopReason::Step);
        assert_eq!(stops[2].seam, Some(SeamKind::FunctionReturn));
        assert_eq!(stops[2].frame_depth, 0, "the finish stop is in the caller");
    }
    // The FrameReturned event reached the session's stream.
    let driver = wired
        .executor
        .debug_driver_mut()
        .expect("driver still installed");
    let mut events = wired.events.borrow().clone();
    events.extend(driver.session_mut().drain_events());
    let returned = events.iter().find_map(|event| match event {
        DebugEvent::FrameReturned(frame) => Some(frame),
        _ => None,
    });
    let frame = returned.expect("a FrameReturned event was emitted");
    assert_eq!(frame.function.as_deref(), Some("f"));
    assert_eq!(frame.route, crate::debugger::ReturnRoute::Completed);
    assert_eq!(frame.status, 0);
    assert_eq!(env_of(&wired.executor, "DONE").as_deref(), Some("1"));
}

/// Design 5.3: a conditional breakpoint's condition is evaluated
/// in the current frame at the would-be hit; the stop commits
/// only when the condition succeeds.
#[test]
fn de02_conditional_breakpoint_evaluates_in_frame() {
    let _span = GateTestSpan::new();
    let (input, path) = sourced_script("condbp", "for i in 1 2 3; do\n  HIT=$i\ndone\n");

    // Condition true on the second iteration only: one stop.
    let file_for_bp = path.clone();
    let mut wired = wired_executor(Box::new(move |session, record| {
        if record.reason == StopReason::Entry {
            let id = session
                .breakpoints_mut()
                .insert_line(file_for_bp.clone(), 2);
            session
                .breakpoints_mut()
                .set_condition(id, Some("[[ $i == 2 ]]".to_string()));
        }
        StopLoopOutcome::Resume(StepArming::Continue)
    }));
    let status = crate::script_driver::run_source(&mut wired.executor, &input, false);
    assert_eq!(status, 0);
    {
        let stops = wired.stops.borrow();
        let bp_stops: Vec<&StopRecord> = stops
            .iter()
            .filter(|stop| matches!(stop.reason, StopReason::Breakpoint(_)))
            .collect();
        assert_eq!(bp_stops.len(), 1, "the condition commits exactly one stop");
        assert_eq!(bp_stops[0].line, Some(2));
    }
    assert_eq!(env_of(&wired.executor, "HIT").as_deref(), Some("3"));

    // Condition never true: the breakpoint never stops.
    let file_for_bp = path.clone();
    let mut wired = wired_executor(Box::new(move |session, record| {
        if record.reason == StopReason::Entry {
            let id = session
                .breakpoints_mut()
                .insert_line(file_for_bp.clone(), 2);
            session
                .breakpoints_mut()
                .set_condition(id, Some("[[ $i == 99 ]]".to_string()));
        }
        StopLoopOutcome::Resume(StepArming::Continue)
    }));
    let status = crate::script_driver::run_source(&mut wired.executor, &input, false);
    assert_eq!(status, 0);
    assert!(
        wired
            .stops
            .borrow()
            .iter()
            .all(|stop| stop.reason == StopReason::Entry),
        "a false condition never stops"
    );
}

/// DE-07: an asynchronous stop requested while the debuggee runs
/// is serviced at the boundary poll — its event precedes the
/// Pause stop it causes.
#[test]
fn de07_async_stop_serviced_at_boundary_poll() {
    let _span = GateTestSpan::new();
    let (input, _path) = sourced_script("async", "A=1\nB=2\nC=3\nD=4\n");

    let mut wired = wired_executor(Box::new(move |session, record| {
        if record.reason == StopReason::Entry {
            session.request_async_stop();
        }
        StopLoopOutcome::Resume(StepArming::Continue)
    }));
    let status = crate::script_driver::run_source(&mut wired.executor, &input, false);
    assert_eq!(status, 0);
    {
        let stops = wired.stops.borrow();
        let pause = stops
            .iter()
            .find(|stop| stop.reason == StopReason::Pause)
            .expect("the async stop was serviced during the run");
        assert_eq!(pause.seam, Some(SeamKind::BoundaryPoll));
    }
    let events = wired.events.borrow();
    let serviced = events
        .iter()
        .position(|event| matches!(event, DebugEvent::AsyncStopServiced))
        .expect("the AsyncStopServiced event was emitted");
    let pause_stop = events
        .iter()
        .position(
            |event| matches!(event, DebugEvent::Stopped(info) if info.reason == StopReason::Pause),
        )
        .expect("the Pause stop followed");
    assert!(serviced < pause_stop, "DE-07 precedes the stop it causes");
    assert_eq!(env_of(&wired.executor, "D").as_deref(), Some("4"));
}

/// Detach at the entry stop drops the session: the debuggee runs
/// on unattached, no further stops are served, and the gate is
/// disarmed.
#[test]
fn detach_at_entry_lets_debuggee_run_free() {
    let _span = GateTestSpan::new();
    let (input, _path) = sourced_script("detach", "A=1\nB=2\n");

    let mut wired = wired_executor(Box::new(move |_session, _record| StopLoopOutcome::Detach));
    let status = crate::script_driver::run_source(&mut wired.executor, &input, false);
    assert_eq!(status, 0);
    assert_eq!(
        wired.stops.borrow().len(),
        1,
        "only the entry stop was served"
    );
    assert!(
        wired.executor.debug_driver().is_none(),
        "the driver is dropped"
    );
    assert!(!crate::debugger::SESSION_ACTIVE.is_armed());
    assert_eq!(env_of(&wired.executor, "B").as_deref(), Some("2"));
}

/// DE-09: the session learns the final status from the finish
/// side — the exit builtin's dispatch for an explicit `exit`, and
/// `finish_shell` for a script that runs off its end; the second
/// report is a no-op.
#[test]
fn de09_session_exited_from_finish_side() {
    let _span = GateTestSpan::new();

    // Explicit exit: the dispatch side speaks first.
    let (input, _path) = sourced_script("exitbuiltin", "X=1\nexit 5\n");
    let mut wired = wired_executor(Box::new(move |_session, _record| {
        StopLoopOutcome::Resume(StepArming::Continue)
    }));
    let status = crate::script_driver::run_source(&mut wired.executor, &input, false);
    assert_eq!(status, 5);
    {
        let driver = wired.executor.debug_driver().expect("driver installed");
        let session = driver.session();
        assert!(session.is_terminated());
        assert_eq!(
            session.termination(),
            Some(crate::debugger::Termination::Exited { status: 5 })
        );
    }
    assert!(!crate::debugger::SESSION_ACTIVE.is_armed());
    // The driver's own finish afterwards must not speak twice.
    crate::script_driver::finish_shell(&mut wired.executor, status, false);
    let driver = wired.executor.debug_driver_mut().expect("driver installed");
    let mut events = wired.events.borrow().clone();
    events.extend(driver.session_mut().drain_events());
    let exited: Vec<&DebugEvent> = events
        .iter()
        .filter(|event| matches!(event, DebugEvent::SessionExited { .. }))
        .collect();
    assert_eq!(exited.len(), 1, "DE-09 is emitted exactly once");

    // Natural end: finish_shell is the only speaker.
    let (input, _path) = sourced_script("naturalend", "X=1\n");
    let mut wired = wired_executor(Box::new(move |_session, _record| {
        StopLoopOutcome::Resume(StepArming::Continue)
    }));
    let status = crate::script_driver::run_source(&mut wired.executor, &input, false);
    assert_eq!(status, 0);
    assert!(
        !wired
            .executor
            .debug_driver()
            .expect("driver installed")
            .session()
            .is_terminated(),
        "not terminated before the driver finish"
    );
    crate::script_driver::finish_shell(&mut wired.executor, status, false);
    let driver = wired.executor.debug_driver_mut().expect("driver installed");
    let mut events = wired.events.borrow().clone();
    events.extend(driver.session_mut().drain_events());
    let exited: Vec<&DebugEvent> = events
        .iter()
        .filter(|event| matches!(event, DebugEvent::SessionExited { .. }))
        .collect();
    assert_eq!(exited.len(), 1, "DE-09 is emitted exactly once");
    assert_eq!(
        driver.session().termination(),
        Some(crate::debugger::Termination::Exited { status: 0 })
    );
}

// ---------------------------------------------------------------------
// P-1 adjudication pins (no session attached): the entry fire's
// skip / force-return semantics against the GNU 5.3.20 oracle
// (probes p1a/p1a2/p1b/p1c/p1e in the cluster's probe directory;
// C source: execute_cmd.c:5383-5407 execute_function).
// ---------------------------------------------------------------------

fn run_plain(script: &str) -> (Executor, i32) {
    let mut executor = Executor::new();
    let status = crate::script_driver::run_source(&mut executor, script, false);
    (executor, status)
}

/// P-1: an entry skip suppresses the RETURN trap for the skipped
/// frame, and the call returns the DEBUG trap's status.
#[test]
fn p1_entry_skip_suppresses_return_trap_and_yields_trap_status() {
    let (executor, status) = run_plain(
        "shopt -s extdebug\n\
         trap 'RT=1' RETURN\n\
         trap 'if [[ ${FUNCNAME[0]-} == f ]]; then false; else true; fi' DEBUG\n\
         f() { BODY=1; }\n\
         f\n\
         RC=$?\n",
    );
    assert_eq!(status, 0);
    assert_eq!(env_of(&executor, "BODY"), None, "the body was skipped");
    assert_eq!(
        env_of(&executor, "RT"),
        None,
        "the RETURN trap did not fire"
    );
    assert_eq!(
        env_of(&executor, "RC").as_deref(),
        Some("1"),
        "the trap's status"
    );
}

/// P-1: the status a skipped frame returns is the trap's own
/// status, not a constant: a status-3 trap yields a status-3
/// call. (The helper runs inside the DEBUG action, so it
/// inherits no RETURN trap — execute_cmd.c:5295 — and the
/// skipped frame fires none either: RT_SEEN stays unset. GNU
/// probe p1a3: `rc=3 RT_SEEN=unset`.)
#[test]
fn p1_entry_skip_status_is_the_traps_status() {
    let (executor, _status) = run_plain(
        "__three() { return 3; }\n\
         shopt -s extdebug\n\
         trap 'RT_SEEN=\"${RT_SEEN}|${FUNCNAME[0]}\"' RETURN\n\
         trap 'if [[ ${FUNCNAME[0]-} == f ]]; then __three; else true; fi' DEBUG\n\
         f() { BODY=1; }\n\
         f\n\
         RC=$?\n",
    );
    assert_eq!(env_of(&executor, "BODY"), None, "the body was skipped");
    assert_eq!(
        env_of(&executor, "RT_SEEN"),
        None,
        "no RETURN trap fired at all"
    );
    assert_eq!(env_of(&executor, "RC").as_deref(), Some("3"));
}

/// P-1: the suppression is structural, not incidental — even a
/// RETURN trap the DEBUG action itself installs while skipping
/// does not fire for the skipped frame (GNU probe p1f: body
/// absent, no fire, status 1).
#[test]
fn p1_entry_skip_never_fires_return_trap_even_when_action_installs_one() {
    let (executor, _status) = run_plain(
        "shopt -s extdebug\n\
         trap 'if [[ ${FUNCNAME[0]-} == f ]]; then trap \"RT=1\" RETURN; false; else true; fi' DEBUG\n\
         f() { BODY=1; }\n\
         f\n\
         RC=$?\n",
    );
    assert_eq!(env_of(&executor, "BODY"), None, "the body was skipped");
    assert_eq!(env_of(&executor, "RT"), None, "the RETURN trap did not fire");
    assert_eq!(env_of(&executor, "RC").as_deref(), Some("1"));
}

/// P-1: with no skip, the body runs and the RETURN trap fires.
#[test]
fn p1_no_skip_fires_return_trap() {
    let (executor, _status) = run_plain(
        "shopt -s extdebug\n\
         trap 'RT=1' RETURN\n\
         trap 'true' DEBUG\n\
         f() { BODY=1; }\n\
         f\n\
         RC=$?\n",
    );
    assert_eq!(env_of(&executor, "BODY").as_deref(), Some("1"));
    assert_eq!(env_of(&executor, "RT").as_deref(), Some("1"));
    assert_eq!(env_of(&executor, "RC").as_deref(), Some("0"));
}

/// P-1: a `return` executed by the DEBUG action at the entry
/// fire (a force-return) still fires the RETURN trap, and the
/// call returns the forced status.
#[test]
fn p1_force_return_fires_return_trap() {
    let (executor, _status) = run_plain(
        "shopt -s extdebug\n\
         trap 'RT=1' RETURN\n\
         trap 'if [[ ${FUNCNAME[0]-} == f && -z ${DONE-} ]]; then DONE=1; return 7; else true; fi' DEBUG\n\
         f() { BODY=1; }\n\
         f\n\
         RC=$?\n",
    );
    assert_eq!(env_of(&executor, "BODY"), None, "the body was skipped");
    assert_eq!(
        env_of(&executor, "RT").as_deref(),
        Some("1"),
        "the RETURN trap fired"
    );
    assert_eq!(env_of(&executor, "RC").as_deref(), Some("7"));
}

/// P-1: without extdebug there is no skip at all — a non-zero
/// entry status under functrace alone leaves the body and the
/// RETURN trap untouched.
#[test]
fn p1_functrace_without_extdebug_does_not_skip() {
    let (executor, _status) = run_plain(
        "set -o functrace\n\
         trap 'RT=1' RETURN\n\
         trap 'if [[ ${FUNCNAME[0]-} == f ]]; then false; else true; fi' DEBUG\n\
         f() { BODY=1; }\n\
         f\n\
         RC=$?\n",
    );
    assert_eq!(env_of(&executor, "BODY").as_deref(), Some("1"));
    assert_eq!(env_of(&executor, "RT").as_deref(), Some("1"));
    assert_eq!(env_of(&executor, "RC").as_deref(), Some("0"));
}
