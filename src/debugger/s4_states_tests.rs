//! Row 9 (S4) acceptance tests: the funnel consult's three
//! states across the design section 4.1 seam faces, the
//! break/continue word forms inside a session, and the S5 KPI
//! harness (ignored by default; numbers archived in the row's
//! REPORT, never asserted).
//!
//! The three states (design 4.4):
//!
//! 1. **No session, no trap action** — the engine's original
//!    path. The funnel pays its gate load and nothing else
//!    exists to consult.
//! 2. **Session attached and armed** — the consult serves
//!    stops: breakpoints, stepping armings, the entry stop.
//! 3. **Session attached but inert** — a DEBUG trap action may
//!    be installed, but no seam entry is enabled and the arming
//!    is `Continue`: the consult must not stop, must not run
//!    its stop-decision machinery, and must not change any
//!    observable behavior — including the trap's own firing
//!    count, which is the most sensitive detector of extra or
//!    missing funnel arrivals.
//!
//! Every face runs in all applicable states and the observable
//! outcome (final status + accumulator variables) must be
//! byte-identical across them; state 2 additionally asserts
//! the face's seam kinds actually arrive.
//!
//! Every test that arms the process gate holds
//! [`super::gate::GATE_TEST_LOCK`] for its whole span.

use std::cell::RefCell;
use std::rc::Rc;

use super::driver::{SessionDriver, StopLoop, StopLoopOutcome};
use super::evaluation::{EvaluationRequest, PowerMode};
use super::event::{DebugEvent, SeamKind, StopReason};
use super::session::{DebugSession, StepArming};
use crate::executor::Executor;

// ---------------------------------------------------------------------
// Harness (the wiring_tests shape, kept local so this module is
// self-contained).
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
struct StopRecord {
    reason: StopReason,
    seam: Option<SeamKind>,
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
            seam: info.seam,
            command_text: info.command_text.clone(),
        };
        self.stops.borrow_mut().push(record.clone());
        self.events.borrow_mut().extend(session.drain_events());
        (self.handler)(session, &record)
    }
}

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

/// The path reaches the driver input **quoted** (the
/// wiring_tests lesson: a bare native path loses its `\`
/// separators to the lexer on Windows and the debuggee never
/// runs).
fn sourced_script(name: &str, script: &str) -> (String, String) {
    let dir = std::env::temp_dir().join(format!("rubash-150-s4-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(format!("{name}.sh"));
    std::fs::write(&path, script).expect("write script");
    let path = path.to_string_lossy().into_owned();
    (format!(". \"{path}\""), path)
}

fn env_of(executor: &Executor, name: &str) -> Option<String> {
    executor.get_env(name).map(|value| value.to_string())
}

fn run_plain(script: &str) -> (Executor, i32) {
    let mut executor = Executor::new();
    let status = crate::script_driver::run_source(&mut executor, script, false);
    (executor, status)
}

/// One face run under a session: `arming` is what the stop loop
/// answers at every stop (the entry stop included).
fn run_wired(script: &str, arming: StepArming) -> (Wired, i32) {
    let mut wired = wired_executor(Box::new(move |_session, _record| {
        StopLoopOutcome::Resume(arming)
    }));
    let status = crate::script_driver::run_source(&mut wired.executor, script, false);
    (wired, status)
}

// ---------------------------------------------------------------------
// The seam faces (design 4.1's distribution, one script each).
// ---------------------------------------------------------------------

struct Face {
    name: &'static str,
    script: String,
    expected_acc: &'static str,
    /// Seam kinds that must arrive at least once under StepInto.
    step_seams: &'static [SeamKind],
    /// Command texts that must never be a stop's text under
    /// StepInto (the never-stop face: a command substitution's
    /// body runs in a driver-less child executor, DE-12).
    absent_texts: &'static [&'static str],
}

fn faces() -> Vec<Face> {
    vec![
        Face {
            name: "simple_command",
            script: "ACC=a\nACC=${ACC}b\n".to_string(),
            expected_acc: "ab",
            step_seams: &[SeamKind::SimpleCommand],
            absent_texts: &[],
        },
        Face {
            name: "conditional",
            script: "if [[ 1 == 1 ]]; then ACC=tt; fi\n".to_string(),
            expected_acc: "tt",
            step_seams: &[SeamKind::Conditional],
            absent_texts: &[],
        },
        Face {
            name: "arithmetic_command",
            script: "(( X = 5 ))\nACC=$X\n".to_string(),
            expected_acc: "5",
            step_seams: &[SeamKind::Arithmetic],
            absent_texts: &[],
        },
        Face {
            name: "time_inner",
            script: "time true\nACC=t\n".to_string(),
            expected_acc: "t",
            step_seams: &[SeamKind::TimeInner],
            absent_texts: &[],
        },
        Face {
            name: "case_head",
            script: "case foo in\nfoo) ACC=matched;;\n*) ACC=other;;\nesac\n".to_string(),
            expected_acc: "matched",
            step_seams: &[SeamKind::CaseHead],
            absent_texts: &[],
        },
        Face {
            name: "arith_for",
            script: "for ((i=0; i<3; i++)); do ACC=${ACC}x; done\n".to_string(),
            expected_acc: "xxx",
            step_seams: &[
                SeamKind::ArithForInit,
                SeamKind::ArithForTest,
                SeamKind::ArithForUpdate,
            ],
            absent_texts: &[],
        },
        Face {
            name: "for_head",
            script: "for i in 1 2 3; do ACC=${ACC}$i; done\n".to_string(),
            expected_acc: "123",
            step_seams: &[SeamKind::ForHead],
            absent_texts: &[],
        },
        Face {
            name: "function_entry",
            script: "f() { ACC=called; }\nf\n".to_string(),
            expected_acc: "called",
            step_seams: &[SeamKind::FunctionEntry],
            absent_texts: &[],
        },
        Face {
            name: "pipeline_element",
            script: "echo hi | read LINE\nACC=after\n".to_string(),
            expected_acc: "after",
            step_seams: &[SeamKind::PipelineElement],
            absent_texts: &[],
        },
        Face {
            name: "while_head_never_stops",
            script: "N=0\nwhile [[ $N -lt 2 ]]; do N=$((N+1)); done\nACC=$N\n".to_string(),
            expected_acc: "2",
            step_seams: &[SeamKind::Conditional],
            absent_texts: &[],
        },
        Face {
            name: "comsub_body_has_no_session",
            script: "ACC=$(echo inner)\n".to_string(),
            expected_acc: "inner",
            step_seams: &[],
            absent_texts: &["echo inner"],
        },
    ]
}

/// The three-state contract, per face: identical observable
/// outcome in state 1 (plain), state 3 (inert session) and
/// state 2 (stepping session); the inert session serves no
/// stop past the entry stop; the stepping session sees the
/// face's seams.
#[test]
fn s4_three_states_agree_on_every_seam_face() {
    let _span = GateTestSpan::new();
    for face in faces() {
        let (plain, plain_status) = run_plain(&face.script);
        assert_eq!(plain_status, 0, "face {}: plain status", face.name);
        assert_eq!(
            env_of(&plain, "ACC").as_deref(),
            Some(face.expected_acc),
            "face {}: plain outcome",
            face.name
        );

        let (inert, inert_status) = run_wired(&face.script, StepArming::Continue);
        assert_eq!(
            inert_status, plain_status,
            "face {}: inert status",
            face.name
        );
        assert_eq!(
            env_of(&inert.executor, "ACC").as_deref(),
            Some(face.expected_acc),
            "face {}: inert outcome equals plain",
            face.name
        );
        {
            let stops = inert.stops.borrow();
            assert_eq!(
                stops.len(),
                1,
                "face {}: an inert session serves only the entry stop, got {stops:?}",
                face.name
            );
            assert_eq!(stops[0].reason, StopReason::Entry);
        }

        let (stepped, step_status) = run_wired(&face.script, StepArming::StepInto);
        assert_eq!(step_status, plain_status, "face {}: step status", face.name);
        assert_eq!(
            env_of(&stepped.executor, "ACC").as_deref(),
            Some(face.expected_acc),
            "face {}: stepped outcome equals plain",
            face.name
        );
        let stops = stepped.stops.borrow();
        for seam in face.step_seams {
            assert!(
                stops.iter().any(|stop| stop.seam == Some(*seam)),
                "face {}: seam {seam:?} never stopped under StepInto; stops: {stops:?}",
                face.name
            );
        }
        for text in face.absent_texts {
            assert!(
                stops
                    .iter()
                    .all(|stop| stop.command_text.as_deref() != Some(text)),
                "face {}: {text:?} must never be a stop (driver-less child); stops: {stops:?}",
                face.name
            );
        }
    }
}

/// The subshell face (design 6.3): a synchronous `( )` runs
/// on the session's own executor — stepping stops inside it —
/// while its writes live and die in the state snapshot. The
/// three states agree on the outcome; only state 2 sees the
/// inner stop.
#[test]
fn s4_subshell_stops_inside_but_writes_stay_in_the_snapshot() {
    let _span = GateTestSpan::new();
    let script = "( ACC=sub )\nACC=outer\n";
    let (plain, plain_status) = run_plain(script);
    assert_eq!(plain_status, 0);
    assert_eq!(env_of(&plain, "ACC").as_deref(), Some("outer"));

    let (inert, inert_status) = run_wired(script, StepArming::Continue);
    assert_eq!(inert_status, 0);
    assert_eq!(env_of(&inert.executor, "ACC").as_deref(), Some("outer"));
    assert_eq!(inert.stops.borrow().len(), 1, "inert: entry stop only");

    let (stepped, step_status) = run_wired(script, StepArming::StepInto);
    assert_eq!(step_status, 0);
    assert_eq!(
        env_of(&stepped.executor, "ACC").as_deref(),
        Some("outer"),
        "the subshell's write stayed in its snapshot"
    );
    assert!(
        stepped
            .stops
            .borrow()
            .iter()
            .any(|stop| stop.command_text.as_deref() == Some("ACC=sub")),
        "stepping stops inside the subshell body"
    );
}

/// The select head face needs a real input file; it runs the
/// same three-state contract separately.
#[test]
fn s4_select_head_three_states() {
    let _span = GateTestSpan::new();
    let dir = std::env::temp_dir().join(format!("rubash-150-s4-select-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let input_path = dir.join("input.txt");
    std::fs::write(&input_path, "1\n").expect("write input");
    let input_path = input_path.to_string_lossy().into_owned();
    let script = format!("select opt in a b; do ACC=$opt; break; done < \"{input_path}\"\n");

    let (plain, plain_status) = run_plain(&script);
    assert_eq!(plain_status, 0);
    assert_eq!(env_of(&plain, "ACC").as_deref(), Some("a"));

    let (inert, inert_status) = run_wired(&script, StepArming::Continue);
    assert_eq!(inert_status, 0);
    assert_eq!(env_of(&inert.executor, "ACC").as_deref(), Some("a"));
    assert_eq!(inert.stops.borrow().len(), 1, "inert: entry stop only");

    let (stepped, step_status) = run_wired(&script, StepArming::StepInto);
    assert_eq!(step_status, 0);
    assert_eq!(env_of(&stepped.executor, "ACC").as_deref(), Some("a"));
    assert!(
        stepped
            .stops
            .borrow()
            .iter()
            .any(|stop| stop.seam == Some(SeamKind::SelectHead)),
        "the select head seam stops under StepInto"
    );
}

/// State 3 with a trap installed: the trap is the user's, the
/// session is inert — firing counts and outcomes must equal
/// the trap-only run exactly, in the inert and the stepping
/// session alike (a stop serve runs no debuggee commands, so
/// it cannot perturb the count).
#[test]
fn s4_inert_session_does_not_perturb_a_debug_trap() {
    let _span = GateTestSpan::new();
    let script = "trap 'TC=$(( ${TC:-0} + 1 ))' DEBUG\n\
                  ACC=\n\
                  for i in 1 2 3; do ACC=${ACC}$i; done\n\
                  f() { ACC=${ACC}f; }\n\
                  f\n";
    let (plain, plain_status) = run_plain(script);
    let plain_tc = env_of(&plain, "TC");
    assert!(plain_tc.is_some(), "the trap fired in the plain run");
    assert_eq!(env_of(&plain, "ACC").as_deref(), Some("123f"));

    for (label, arming) in [
        ("inert", StepArming::Continue),
        ("stepping", StepArming::StepInto),
    ] {
        let (wired, status) = run_wired(script, arming);
        assert_eq!(status, plain_status, "{label}: status");
        assert_eq!(
            env_of(&wired.executor, "ACC").as_deref(),
            Some("123f"),
            "{label}: outcome"
        );
        assert_eq!(
            env_of(&wired.executor, "TC"),
            plain_tc,
            "{label}: trap firing count is the plain run's"
        );
    }
}

/// The armed-but-inert shapes of the table itself: every entry
/// disabled, and watch entries only (watch stop-and-diff is
/// serviced at stops, never at seams — a watch-only session
/// has nothing that can stop at a seam). Both must behave
/// exactly like the plain run, entry stop aside.
#[test]
fn s4_disabled_and_watch_only_tables_are_inert() {
    let _span = GateTestSpan::new();
    let (input, path) = sourced_script("inert", "ACC=1\nACC=2\nTAIL=end\n");

    // Every entry disabled.
    let file_for_bp = path.clone();
    let mut wired = wired_executor(Box::new(move |session, record| {
        if record.reason == StopReason::Entry {
            let id = session
                .breakpoints_mut()
                .insert_line(file_for_bp.clone(), 2);
            session.breakpoints_mut().set_enabled(id, false);
        }
        StopLoopOutcome::Resume(StepArming::Continue)
    }));
    let status = crate::script_driver::run_source(&mut wired.executor, &input, false);
    assert_eq!(status, 0);
    assert_eq!(wired.stops.borrow().len(), 1, "disabled entries never stop");
    assert_eq!(env_of(&wired.executor, "ACC").as_deref(), Some("2"));

    // Watch entries only.
    let mut wired = wired_executor(Box::new(move |session, record| {
        if record.reason == StopReason::Entry {
            session.breakpoints_mut().insert_watch("ACC");
        }
        StopLoopOutcome::Resume(StepArming::Continue)
    }));
    let status = crate::script_driver::run_source(&mut wired.executor, &input, false);
    assert_eq!(status, 0);
    assert_eq!(
        wired.stops.borrow().len(),
        1,
        "a watch-only session stops nowhere at seams"
    );
    assert_eq!(env_of(&wired.executor, "TAIL").as_deref(), Some("end"));
}

/// B-3 (WP-1 inventory; GNU execute_cmd.c:3053-3059): under
/// extdebug, a DEBUG trap whose status is non-zero at the
/// `for` head skips the iteration — a `continue` — and when
/// the skipped iteration is the last, the loop's status is
/// the trap's status. Oracle: probes/b3.sh against GNU bash
/// 5.3.20 prints `S=[] FORST=1`. Pinned plain and with an
/// inert session attached: the consult must not change the
/// face (the extdebug veto is decided before it, design 4.2).
#[test]
fn s4_extdebug_for_head_skip_is_a_continue_with_trap_status() {
    let _span = GateTestSpan::new();
    let script = "shopt -s extdebug\n\
                  trap 'if [[ $BASH_COMMAND == for* ]]; then false; else true; fi' DEBUG\n\
                  S=\n\
                  for i in 1 2 3; do S=${S}$i; done\n\
                  FORST=$?\n";
    let (plain, status) = run_plain(script);
    assert_eq!(status, 0);
    assert_eq!(env_of(&plain, "S").as_deref(), Some(""));
    assert_eq!(env_of(&plain, "FORST").as_deref(), Some("1"));

    let (inert, inert_status) = run_wired(script, StepArming::Continue);
    assert_eq!(inert_status, 0);
    assert_eq!(env_of(&inert.executor, "S").as_deref(), Some(""));
    assert_eq!(env_of(&inert.executor, "FORST").as_deref(), Some("1"));
}

// ---------------------------------------------------------------------
// break / continue word forms inside a session.
// ---------------------------------------------------------------------

/// Stepping through a loop whose body breaks and continues:
/// the `break` and `continue` commands are ordinary
/// simple-command seams (the stop lands before them), the
/// flow after them follows the loop's own semantics, and the
/// outcome equals the plain run.
#[test]
fn s4_step_into_loop_with_break_and_continue() {
    let _span = GateTestSpan::new();
    let script = "ACC=\n\
                  for i in 1 2 3 4 5; do\n\
                    if [[ $i == 2 ]]; then continue; fi\n\
                    if [[ $i == 4 ]]; then break; fi\n\
                    ACC=${ACC}$i\n\
                  done\n";
    let (plain, plain_status) = run_plain(script);
    assert_eq!(plain_status, 0);
    assert_eq!(env_of(&plain, "ACC").as_deref(), Some("13"));

    let (stepped, status) = run_wired(script, StepArming::StepInto);
    assert_eq!(status, 0);
    assert_eq!(env_of(&stepped.executor, "ACC").as_deref(), Some("13"));
    let stops = stepped.stops.borrow();
    let texts: Vec<&str> = stops
        .iter()
        .filter_map(|stop| stop.command_text.as_deref())
        .collect();
    let continues = texts.iter().filter(|text| **text == "continue").count();
    let breaks = texts.iter().filter(|text| **text == "break").count();
    assert_eq!(continues, 1, "one stop at the continue command: {texts:?}");
    assert_eq!(breaks, 1, "one stop at the break command: {texts:?}");
    let continue_at = texts.iter().position(|text| *text == "continue").unwrap();
    let break_at = texts.iter().position(|text| *text == "break").unwrap();
    assert!(continue_at < break_at, "iteration order preserved");
}

/// `break 2` unwinds two loop levels; stepping sees the one
/// command and the post-loop flow, nothing in between.
#[test]
fn s4_step_into_break_two_levels() {
    let _span = GateTestSpan::new();
    let script = "ACC=\n\
                  for a in 1 2; do\n\
                    for b in 1 2 3; do\n\
                      if [[ $b == 2 ]]; then break 2; fi\n\
                      ACC=${ACC}$a$b\n\
                    done\n\
                  done\n\
                  TAIL=end\n";
    let (plain, plain_status) = run_plain(script);
    assert_eq!(plain_status, 0);
    assert_eq!(env_of(&plain, "ACC").as_deref(), Some("11"));

    let (stepped, status) = run_wired(script, StepArming::StepInto);
    assert_eq!(status, 0);
    assert_eq!(env_of(&stepped.executor, "ACC").as_deref(), Some("11"));
    assert_eq!(env_of(&stepped.executor, "TAIL").as_deref(), Some("end"));
    let stops = stepped.stops.borrow();
    let break_stops = stops
        .iter()
        .filter(|stop| stop.command_text.as_deref() == Some("break 2"))
        .count();
    assert_eq!(break_stops, 1, "one stop at `break 2`: {stops:?}");
}

// ---------------------------------------------------------------------
// Control unwinds typed at a stop (explicit evaluation). The
// evaluation layer's rule (design 7.4; WP-7) is that a typed
// `exit` / `return` / `break` / `continue` propagates as the
// debuggee's own flow. A propagated unwind IS a resume: the
// session must be Running again as the unwind leaves the
// serve — the same resume-before-unwind discipline the
// ForceReturn route follows — or every later seam would find
// a stopped session and no stop could ever land again.
// ---------------------------------------------------------------------

/// A stop loop that, at the first breakpoint stop matching
/// `at_line`, asks for one explicit evaluation of `text`, and
/// answers Continue everywhere else. Records every stop line.
struct EvalOnceLoop {
    file: String,
    bp_lines: Vec<u32>,
    at_line: u32,
    text: String,
    fired: Rc<RefCell<bool>>,
    stops: Rc<RefCell<Vec<StopRecord>>>,
    events: Rc<RefCell<Vec<DebugEvent>>>,
}

impl StopLoop for EvalOnceLoop {
    fn serve(&mut self, session: &mut DebugSession) -> StopLoopOutcome {
        let info = session
            .last_stop()
            .cloned()
            .expect("a stop-loop serve implies a stop");
        let record = StopRecord {
            reason: info.reason,
            seam: info.seam,
            command_text: info.command_text.clone(),
        };
        self.stops.borrow_mut().push(record);
        self.events.borrow_mut().extend(session.drain_events());
        if info.reason == StopReason::Entry {
            for line in &self.bp_lines {
                session
                    .breakpoints_mut()
                    .insert_line(self.file.clone(), *line);
            }
            return StopLoopOutcome::Resume(StepArming::Continue);
        }
        if matches!(info.reason, StopReason::Breakpoint(_))
            && info.location.as_ref().map(|loc| loc.line) == Some(self.at_line)
            && !*self.fired.borrow()
        {
            *self.fired.borrow_mut() = true;
            return StopLoopOutcome::Evaluate(EvaluationRequest {
                mode: PowerMode::Explicit,
                text: self.text.clone(),
                frame: None,
            });
        }
        StopLoopOutcome::Resume(StepArming::Continue)
    }
}

struct EvalRun {
    executor: Executor,
    stops: Rc<RefCell<Vec<StopRecord>>>,
    events: Rc<RefCell<Vec<DebugEvent>>>,
    status: i32,
}

fn run_eval_once(
    name: &str,
    script: &str,
    bp_lines: Vec<u32>,
    at_line: u32,
    text: &str,
) -> EvalRun {
    let (input, path) = sourced_script(name, script);
    let stops = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let driver = SessionDriver::new(
        DebugSession::new(),
        Box::new(EvalOnceLoop {
            file: path,
            bp_lines,
            at_line,
            text: text.to_string(),
            fired: Rc::new(RefCell::new(false)),
            stops: stops.clone(),
            events: events.clone(),
        }),
    );
    let mut executor = Executor::new();
    executor.install_debug_session(driver);
    let status = crate::script_driver::run_source(&mut executor, &input, false);
    // Events emitted after the last served stop (a frame return
    // reported during an unwind, the exit note) are still queued
    // on the session; collect them so the run's event record is
    // complete.
    if let Some(driver) = executor.debug_driver_mut() {
        events
            .borrow_mut()
            .extend(driver.session_mut().drain_events());
    }
    EvalRun {
        executor,
        stops,
        events,
        status,
    }
}

/// Breakpoint stops are counted on the session's event stream:
/// exactly one `Stopped` event is emitted per stop transition,
/// while a stop-loop serve happens once more after each
/// evaluation (the same stop, served again) — serve records
/// would double-count an evaluated stop.
fn bp_stop_count(run: &EvalRun) -> usize {
    run.events
        .borrow()
        .iter()
        .filter(|event| {
            matches!(
                event,
                DebugEvent::Stopped(info) if matches!(info.reason, StopReason::Breakpoint(_))
            )
        })
        .count()
}

const LOOP_SCRIPT: &str = "ACC=\n\
                           for i in 1 2 3; do\n\
                             ACC=${ACC}$i\n\
                             MARK=hit$i\n\
                           done\n\
                           TAIL=after\n\
                           ACC=${ACC}Z\n";

/// `break` typed at a stop inside a loop body: the loop
/// breaks (the pending command is preempted, as the command
/// typed in the script would preempt it), execution continues
/// after the loop, and the session is alive — the breakpoint
/// after the loop still stops. The typed text is `break 1`:
/// a bare single word is a print-shaped request under the
/// WP-7 routing (it expands, it does not execute), so the
/// executable spelling carries its count, as in the script.
#[test]
fn s4_eval_break_unwinds_the_loop_and_the_session_lives() {
    let _span = GateTestSpan::new();
    let run = run_eval_once("evalbreak", LOOP_SCRIPT, vec![4, 7], 4, "break 1");
    assert_eq!(run.status, 0);
    assert_eq!(env_of(&run.executor, "ACC").as_deref(), Some("1Z"));
    assert_eq!(
        env_of(&run.executor, "MARK"),
        None,
        "the pending command at the stop was preempted"
    );
    assert_eq!(env_of(&run.executor, "TAIL").as_deref(), Some("after"));
    assert_eq!(
        bp_stop_count(&run),
        2,
        "the line-4 stop, then the line-7 stop after the loop: {:?}",
        run.stops.borrow()
    );
}

/// `continue` typed at the same stop: the iteration's rest is
/// skipped, the loop runs on, and later breakpoint stops land
/// normally.
#[test]
fn s4_eval_continue_skips_the_iteration_and_the_session_lives() {
    let _span = GateTestSpan::new();
    let run = run_eval_once("evalcont", LOOP_SCRIPT, vec![4, 7], 4, "continue 1");
    assert_eq!(run.status, 0);
    assert_eq!(env_of(&run.executor, "ACC").as_deref(), Some("123Z"));
    assert_eq!(env_of(&run.executor, "MARK").as_deref(), Some("hit3"));
    assert_eq!(
        bp_stop_count(&run),
        4,
        "line 4 on all three iterations, then line 7: {:?}",
        run.stops.borrow()
    );
}

/// `return 7` typed at a stop inside a function: the frame
/// returns with the typed status, its return is reported
/// (DE-04), and the caller's later breakpoint still stops.
#[test]
fn s4_eval_return_unwinds_the_frame_and_the_session_lives() {
    let _span = GateTestSpan::new();
    let script = "f() {\n\
                  INNER=before\n\
                  INNER=after\n\
                  }\n\
                  f\n\
                  RC=$?\n\
                  TAIL=end\n";
    let run = run_eval_once("evalret", script, vec![3, 7], 3, "return 7");
    assert_eq!(run.status, 0);
    assert_eq!(env_of(&run.executor, "INNER").as_deref(), Some("before"));
    assert_eq!(env_of(&run.executor, "RC").as_deref(), Some("7"));
    assert_eq!(env_of(&run.executor, "TAIL").as_deref(), Some("end"));
    assert_eq!(
        bp_stop_count(&run),
        2,
        "the line-3 stop, then the caller's line-7 stop: {:?}",
        run.stops.borrow()
    );
    // The frame's return was reported to the session (DE-04) —
    // pre-fix the note found a stopped session and the finish
    // face of the event stayed dark.
    let events = run.events.borrow();
    assert!(
        events.iter().any(|event| matches!(
            event,
            DebugEvent::FrameReturned(frame)
                if frame.function.as_deref() == Some("f") && frame.status == 7
        )),
        "a FrameReturned(f, 7) event reached the session: {events:?}"
    );
}

/// `exit 5` typed at a stop: the run ends with the typed
/// status and the session terminates through the usual
/// finish-side note (DE-09), gate disarmed.
#[test]
fn s4_eval_exit_ends_the_run_and_terminates_the_session() {
    let _span = GateTestSpan::new();
    let run = run_eval_once("evalexit", "X=1\nY=2\nZ=3\n", vec![2], 2, "exit 5");
    assert_eq!(run.status, 5);
    assert_eq!(
        env_of(&run.executor, "Z"),
        None,
        "the run ended at the stop"
    );
    // The typed `exit` walks the exit builtin's own dispatch,
    // whose finish-side note (DE-09) terminates the session
    // and disarms the gate during the run itself.
    let driver = run.executor.debug_driver().expect("driver installed");
    assert!(
        driver.session().is_terminated(),
        "the session terminated on the typed exit"
    );
    assert!(!crate::debugger::SESSION_ACTIVE.is_armed());
}

// ---------------------------------------------------------------------
// S5 KPI harness. Ignored: run explicitly, release mode.
// Numbers are archived in the row-9 REPORT with their caliber;
// nothing here asserts a timing bound.
// ---------------------------------------------------------------------

/// KPI: a 4500-line script under three session shapes —
/// plain (no session), inert session (attached, Continue, no
/// entries), and the stop-loop echo (StepInto: every line
/// stops and the loop immediately resumes — the request/
/// response ping-pong the DAP adapter of row 10 will ride
/// on; the adapter itself does not exist yet, so this is the
/// protocol-shaped surface that does).
#[test]
#[ignore = "KPI harness (row 9): run explicitly with --release --nocapture"]
fn s4_kpi_4500_line_script_three_shapes() {
    let _span = GateTestSpan::new();
    let mut script = String::from("ACC=0\n");
    for _ in 0..4498 {
        script.push_str("ACC=$((ACC+1))\n");
    }
    script.push_str("DONE=$ACC\n");

    let started = std::time::Instant::now();
    let (plain, plain_status) = run_plain(&script);
    let plain_elapsed = started.elapsed();
    assert_eq!(plain_status, 0);
    assert_eq!(env_of(&plain, "DONE").as_deref(), Some("4498"));

    let started = std::time::Instant::now();
    let (inert, inert_status) = run_wired(&script, StepArming::Continue);
    let inert_elapsed = started.elapsed();
    assert_eq!(inert_status, 0);
    assert_eq!(env_of(&inert.executor, "DONE").as_deref(), Some("4498"));
    assert_eq!(inert.stops.borrow().len(), 1);

    let started = std::time::Instant::now();
    let (echo, echo_status) = run_wired(&script, StepArming::StepInto);
    let echo_elapsed = started.elapsed();
    assert_eq!(echo_status, 0);
    assert_eq!(env_of(&echo.executor, "DONE").as_deref(), Some("4498"));
    let echo_stops = echo.stops.borrow().len();

    println!(
        "KPI 4500 lines: plain={plain_elapsed:?} inert={inert_elapsed:?} \
         step_echo={echo_elapsed:?} step_echo_stops={echo_stops}"
    );
}
