//! Row 7 acceptance fixtures (WP-4 + WP-8): cross-function
//! stepping (debugger design section 8, scenario 4), the
//! `return [status]` force-return, and the pending function
//! breakpoint's end-to-end life — set before the definition
//! exists, resolved when the definition's reader unit registers,
//! fired at the function's first call.
//!
//! The harness mirrors `wiring_tests`' (a scripted stop loop over
//! a sourced temp file), with the one correction the row 6 CI
//! history bought: the path is quoted in the driver input, so the
//! sourced identity is the temp path verbatim on every platform.
//!
//! Every test that arms the process gate holds
//! [`super::gate::GATE_TEST_LOCK`] for its whole span.

use std::cell::RefCell;
use std::rc::Rc;

use super::breakpoints::BreakpointResolution;
use super::driver::{SessionDriver, StopLoop, StopLoopOutcome};
use super::event::{DebugEvent, ReturnRoute, SeamKind, StopReason};
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
/// that sources it — path quoted, per the row 6 CI lesson — plus
/// the file's path string, which is the identity the engine will
/// report for it.
fn sourced_script(name: &str, script: &str) -> (String, String) {
    let dir = std::env::temp_dir().join(format!("rubash-150-row7-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(format!("{name}.sh"));
    std::fs::write(&path, script).expect("write script");
    let path = path.to_string_lossy().into_owned();
    (format!(". \"{path}\""), path)
}

fn env_of(executor: &Executor, name: &str) -> Option<String> {
    executor.get_env(name).map(|value| value.to_string())
}

/// Design section 8, scenario 4 (function-crossing steps): a line
/// breakpoint parks on the call line; step-into enters the callee
/// (entry seam, then body); step-over runs a nested call without
/// stopping inside it; finish stops in the caller at the frame's
/// return site; continue runs the script out.
#[test]
fn scenario4_step_next_finish_across_function_calls() {
    let _span = GateTestSpan::new();
    let script = "\
inner() {
  INNER_RAN=1
}
outer() {
  inner
  AFTER_INNER=1
}
outer
DONE=1
";
    let (input, path) = sourced_script("scenario4", script);
    let file = path.clone();
    let mut serve_count = 0usize;
    let mut wired = wired_executor(Box::new(move |session, record| {
        let index = serve_count;
        serve_count += 1;
        match index {
            0 => {
                assert_eq!(record.reason, StopReason::Entry);
                session.set_line_breakpoint(file.clone(), 8);
                StopLoopOutcome::Resume(StepArming::Continue)
            }
            1 => StopLoopOutcome::Resume(StepArming::StepInto),
            2 => StopLoopOutcome::Resume(StepArming::StepInto),
            3 => StopLoopOutcome::Resume(StepArming::StepOver { max_depth: 1 }),
            4 => StopLoopOutcome::Resume(StepArming::Finish { frame_depth: 1 }),
            _ => StopLoopOutcome::Resume(StepArming::Continue),
        }
    }));
    let status = crate::script_driver::run_source(&mut wired.executor, &input, false);
    assert_eq!(status, 0);
    let stops = wired.stops.borrow();
    assert_eq!(
        stops.len(),
        6,
        "entry, breakpoint, entry seam, body, after-call, return site: {stops:?}"
    );
    assert_eq!(stops[0].reason, StopReason::Entry);
    // The line breakpoint parks on the call line.
    assert!(matches!(stops[1].reason, StopReason::Breakpoint(_)));
    assert_eq!(stops[1].file.as_deref(), Some(path.as_str()));
    assert_eq!(stops[1].line, Some(8));
    assert_eq!(stops[1].frame_depth, 0);
    // Step-into enters outer: the entry seam, one frame deep.
    // Its reported line is the *definition* line (4): at entry the
    // engine stamps the current line with the definition line —
    // the same stamp the RETURN trap path documents — not the
    // call line.
    assert_eq!(stops[2].reason, StopReason::Step);
    assert_eq!(stops[2].seam, Some(SeamKind::FunctionEntry));
    assert_eq!(stops[2].line, Some(4));
    assert_eq!(stops[2].frame_depth, 1);
    // Step-into again: outer's body, its first line — the inner
    // call.
    assert_eq!(stops[3].reason, StopReason::Step);
    assert_eq!(stops[3].line, Some(5));
    assert_eq!(stops[3].frame_depth, 1);
    // Step-over crosses the inner call: inner's body (line 2,
    // depth 2) never stops; the next stop is the line after.
    assert_eq!(stops[4].reason, StopReason::Step);
    assert_eq!(stops[4].line, Some(6));
    assert_eq!(stops[4].frame_depth, 1);
    // Finish: outer returns; the session stops in the caller at
    // the return site, at the call line, depth 0.
    assert_eq!(stops[5].reason, StopReason::Step);
    assert_eq!(stops[5].seam, Some(SeamKind::FunctionReturn));
    assert_eq!(stops[5].line, Some(8));
    assert_eq!(stops[5].frame_depth, 0);
    drop(stops);
    // The step-over really ran inner, and the whole script ran out.
    assert_eq!(env_of(&wired.executor, "INNER_RAN").as_deref(), Some("1"));
    assert_eq!(env_of(&wired.executor, "AFTER_INNER").as_deref(), Some("1"));
    assert_eq!(env_of(&wired.executor, "DONE").as_deref(), Some("1"));
    // DE-04 reported both frames' returns.
    let events = wired.events.borrow();
    let returned: Vec<&DebugEvent> = events
        .iter()
        .filter(|event| matches!(event, DebugEvent::FrameReturned(_)))
        .collect();
    assert_eq!(
        returned.len(),
        2,
        "inner and outer both reported: {events:?}"
    );
    assert!(events.iter().any(|event| matches!(
        event,
        DebugEvent::FrameReturned(frame)
            if frame.function.as_deref() == Some("outer")
                && frame.frame_depth == 1
                && frame.status == 0
                && frame.route == ReturnRoute::Completed
    )));
}

/// Design 6.1, `return [status]`: parked inside `inner`, the stop
/// loop forces a return with status 7. The rest of inner's body
/// never runs, the frame returns with 7 (DE-04, route Completed —
/// the unwind is the engine's own return path), the session stops
/// in the caller at the return point, and the caller's `$?` is 7.
#[test]
fn force_return_unwinds_the_frame_and_stops_in_the_caller() {
    let _span = GateTestSpan::new();
    let script = "\
inner() {
  BEFORE=1
  MARK=hit
  AFTER=never
}
outer() {
  inner
  RC=$?
}
outer
FINAL=done
";
    let (input, path) = sourced_script("forcereturn", script);
    let file = path.clone();
    let mut serve_count = 0usize;
    let mut wired = wired_executor(Box::new(move |session, record| {
        let index = serve_count;
        serve_count += 1;
        match index {
            0 => {
                assert_eq!(record.reason, StopReason::Entry);
                session.set_line_breakpoint(file.clone(), 3);
                StopLoopOutcome::Resume(StepArming::Continue)
            }
            1 => StopLoopOutcome::ForceReturn(Some(7)),
            _ => StopLoopOutcome::Resume(StepArming::Continue),
        }
    }));
    let status = crate::script_driver::run_source(&mut wired.executor, &input, false);
    assert_eq!(status, 0);
    let stops = wired.stops.borrow();
    assert_eq!(
        stops.len(),
        3,
        "entry, the line-3 stop, the return-point stop: {stops:?}"
    );
    assert!(matches!(stops[1].reason, StopReason::Breakpoint(_)));
    assert_eq!(stops[1].line, Some(3));
    assert_eq!(stops[1].frame_depth, 2, "inner runs one frame inside outer");
    assert_eq!(stops[2].reason, StopReason::Step);
    assert_eq!(stops[2].seam, Some(SeamKind::FunctionReturn));
    assert_eq!(stops[2].line, Some(7), "the return point is the call line");
    assert_eq!(stops[2].frame_depth, 1, "the caller is outer's frame");
    drop(stops);
    assert_eq!(env_of(&wired.executor, "BEFORE").as_deref(), Some("1"));
    assert_eq!(
        env_of(&wired.executor, "MARK"),
        None,
        "the stop was before line 3: it never executed"
    );
    assert_eq!(
        env_of(&wired.executor, "AFTER"),
        None,
        "the forced return skipped the rest of the body"
    );
    assert_eq!(
        env_of(&wired.executor, "RC").as_deref(),
        Some("7"),
        "the caller observes the forced status"
    );
    assert_eq!(env_of(&wired.executor, "FINAL").as_deref(), Some("done"));
    let events = wired.events.borrow();
    assert!(events.iter().any(|event| matches!(
        event,
        DebugEvent::FrameReturned(frame)
            if frame.function.as_deref() == Some("inner")
                && frame.frame_depth == 2
                && frame.status == 7
                && frame.route == ReturnRoute::Completed
    )));
}

/// Design 5.2 end to end: at the entry stop the function is not
/// defined anywhere the session knows — the breakpoint is
/// pending. When the definition's reader unit registers, it
/// resolves (DE-11, verified), and the function's first call
/// stops at its entry seam.
#[test]
fn pending_function_breakpoint_resolves_at_definition_and_fires() {
    let _span = GateTestSpan::new();
    let script = "\
echo start
helper() {
  HELPED=1
}
helper
DONE=1
";
    let (input, path) = sourced_script("pendingfn", script);
    let inserted: Rc<RefCell<Vec<(super::breakpoints::BreakpointId, BreakpointResolution)>>> =
        Rc::new(RefCell::new(Vec::new()));
    let inserted_in_handler = inserted.clone();
    let mut wired = wired_executor(Box::new(move |session, record| {
        if record.reason == StopReason::Entry {
            let fn_id = session.set_function_breakpoint("helper");
            let resolution = session
                .breakpoints()
                .get(fn_id)
                .expect("just inserted")
                .resolution
                .clone();
            inserted_in_handler.borrow_mut().push((fn_id, resolution));
        }
        StopLoopOutcome::Resume(StepArming::Continue)
    }));
    let status = crate::script_driver::run_source(&mut wired.executor, &input, false);
    assert_eq!(status, 0);
    // The insertion adjudicated Pending: at the entry stop only
    // the first reader unit (`echo start`) has been parsed.
    {
        let inserted = inserted.borrow();
        assert_eq!(inserted.len(), 1);
        assert_eq!(inserted[0].1, BreakpointResolution::Pending);
    }
    let stops = wired.stops.borrow();
    assert_eq!(
        stops.len(),
        2,
        "entry, then the function-entry stop: {stops:?}"
    );
    assert!(matches!(stops[1].reason, StopReason::FunctionBreakpoint(_)));
    assert_eq!(stops[1].seam, Some(SeamKind::FunctionEntry));
    assert_eq!(
        stops[1].line,
        Some(2),
        "the entry seam reports the definition line, not the call line"
    );
    assert_eq!(stops[1].file.as_deref(), Some(path.as_str()));
    assert_eq!(stops[1].frame_depth, 1);
    drop(stops);
    // The entry resolved Verified, reported exactly once (DE-11).
    let events = wired.events.borrow();
    let resolved: Vec<&DebugEvent> = events
        .iter()
        .filter(|event| matches!(event, DebugEvent::BreakpointResolved { .. }))
        .collect();
    assert_eq!(resolved.len(), 1, "exactly one DE-11 report: {events:?}");
    let inserted_events = inserted.borrow();
    for (id, _) in inserted_events.iter() {
        assert!(
            events.iter().any(|event| matches!(
                event,
                DebugEvent::BreakpointResolved { id: rid, resolution }
                    if rid == id && *resolution == BreakpointResolution::Verified
            )),
            "entry {id} reported verified"
        );
    }
    drop(events);
    drop(inserted_events);
    assert_eq!(env_of(&wired.executor, "HELPED").as_deref(), Some("1"));
    assert_eq!(env_of(&wired.executor, "DONE").as_deref(), Some("1"));
    // And the session's final record agrees.
    let driver = wired.executor.debug_driver().expect("driver attached");
    for (id, _) in inserted.borrow().iter() {
        assert_eq!(
            driver
                .session()
                .breakpoints()
                .get(*id)
                .expect("entry")
                .resolution,
            BreakpointResolution::Verified
        );
    }
}

/// A line breakpoint on a function *definition* line, pinned to
/// the substrate's real shape: no seam fires while the definition
/// executes (the executor's seam admission skips definition
/// commands, as GNU's dispatch does), so nothing stops there —
/// the breakpoint binds through the entry seam, whose line stamp
/// is the definition line, and fires when the function is
/// called. Its resolution is Verified: the entry seam is a real
/// stop point for this line.
#[test]
fn line_breakpoint_on_a_definition_line_fires_at_the_entry_seam() {
    let _span = GateTestSpan::new();
    let script = "\
worker() {
  WORKED=1
}
BEFORE_CALL=1
worker
DONE=1
";
    let (input, path) = sourced_script("defline", script);
    let file = path.clone();
    let mut wired = wired_executor(Box::new(move |session, record| {
        if record.reason == StopReason::Entry {
            session.set_line_breakpoint(file.clone(), 1);
        }
        StopLoopOutcome::Resume(StepArming::Continue)
    }));
    let status = crate::script_driver::run_source(&mut wired.executor, &input, false);
    assert_eq!(status, 0);
    let stops = wired.stops.borrow();
    assert_eq!(
        stops.len(),
        2,
        "no stop while the definition executes; one stop at the call's entry seam: {stops:?}"
    );
    assert!(matches!(stops[1].reason, StopReason::Breakpoint(_)));
    assert_eq!(stops[1].seam, Some(SeamKind::FunctionEntry));
    assert_eq!(stops[1].line, Some(1));
    assert_eq!(stops[1].file.as_deref(), Some(path.as_str()));
    assert_eq!(stops[1].frame_depth, 1);
    drop(stops);
    assert_eq!(env_of(&wired.executor, "WORKED").as_deref(), Some("1"));
    assert_eq!(env_of(&wired.executor, "DONE").as_deref(), Some("1"));
    let driver = wired.executor.debug_driver().expect("driver attached");
    let entry = driver
        .session()
        .breakpoints()
        .iter()
        .next()
        .expect("the line entry");
    assert_eq!(entry.resolution, BreakpointResolution::Verified);
}
