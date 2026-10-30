//! Row 8 acceptance fixtures (WP-7): the design section 8
//! scenario 7 battery — safe forms pass, the rejected forms of
//! the section 7.5 list are refused item by item with the
//! compliant diagnostic, explicit evaluation runs at full power,
//! and write-back lands through the real assignment path — plus
//! the unit battery pinning the whitelist/rejection
//! correspondence one form at a time.
//!
//! Every test that arms the process gate holds
//! [`super::gate::GATE_TEST_LOCK`] for its whole span.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;

use super::driver::{SessionDriver, StopLoop, StopLoopOutcome};
use super::evaluation::{
    check_implicit, parse_failure, single_expansion_word, DiagnosticKind, EvaluationOutcome,
    EvaluationRequest, EvaluationResult, PowerMode, RejectionKind,
};
use super::event::StopReason;
use super::session::{DebugSession, StepArming};
use crate::executor::Executor;

fn parse(text: &str) -> crate::parser::Ast {
    let tokens = crate::lexer::tokenize(text);
    crate::parser::parse(&tokens)
}

fn implicit_ok(text: &str) {
    let ast = parse(text);
    assert!(
        parse_failure(&ast).is_none(),
        "{text}: the battery's allowed forms must parse"
    );
    match check_implicit(&ast, text) {
        Ok(word) => assert_eq!(
            Some(word),
            single_expansion_word(&ast),
            "{text}: the admitted word is the shape test's word"
        ),
        Err(rejection) => panic!("{text}: expected admission, got {rejection:?}"),
    }
}

fn implicit_rejects(text: &str, kind: RejectionKind) {
    let ast = parse(text);
    match check_implicit(&ast, text) {
        Ok(word) => panic!("{text}: expected rejection {kind:?}, admitted {word:?}"),
        Err(rejection) => {
            assert_eq!(rejection.kind, kind, "{text}: rejection class");
            assert!(!rejection.form.is_empty(), "{text}: the form is named");
            let diagnostic = rejection.diagnostic();
            assert_eq!(diagnostic.kind, DiagnosticKind::Expansion);
            assert!(
                diagnostic
                    .message
                    .starts_with("implicit evaluation skipped: "),
                "{text}: diagnostic shape: {}",
                diagnostic.message
            );
        }
    }
}

/// Design 7.5's allowed list, item by item: plain and positional
/// parameters, array subscripts, `${!x}` indirection, casemod,
/// `${x@Q}`, pattern and substring operations, assignment-free
/// arithmetic — including forms nested inside an allowed form's
/// operand, and dynamic faces read at a stop.
#[test]
fn implicit_whitelist_admits_the_read_only_forms() {
    for text in [
        "$x",
        "x",
        "hello",
        "$1",
        "${10}",
        "$0",
        "$?",
        "$#",
        "$@",
        "$*",
        "${arr[2]}",
        "${arr[@]}",
        "${arr[*]}",
        "${arr[$i]}",
        "${arr[$((i+1))]}",
        "${arr[-1]}",
        "${MAP[k1]}",
        "${MAP[\"a=b\"]}",
        "${!x}",
        "${!arr[@]}",
        "${!x[@]}",
        "${!pre*}",
        "${#x}",
        "${#arr[@]}",
        "${x^^}",
        "${x,,}",
        "${x^}",
        "${x~~}",
        "${x@Q}",
        "${x@a}",
        "${x@P}",
        "${x#pat}",
        "${x##pat}",
        "${x%pat}",
        "${x%%pat}",
        "${x/pat/rep}",
        "${x/pat}",
        "${x//pat/rep}",
        "${x:1:2}",
        "${x: -1}",
        "${x:-d}",
        "${x-d}",
        "${x:+a}",
        "${x:-a=b}",
        "${x:-'a b'}",
        "${x:-y}z",
        "a$b",
        "\"${x}\"",
        "'$x'",
        "~",
        "~/x",
        "{x}",
        "$((x+1))",
        "$((RANDOM))",
        "$((x > 1 ? 2 : 3))",
        "$((x==1))",
        "$((x>=1))",
        "$((x!=1))",
        "$((x, y))",
        "${EPOCHSECONDS}",
        "${BASH_COMMAND}",
    ] {
        implicit_ok(text);
    }
}

/// Design 7.5's rejected list, item by item: every form has its
/// own pin, with its rejection class.
#[test]
fn implicit_whitelist_rejects_each_listed_form() {
    for (text, kind) in [
        ("$(date)", RejectionKind::CommandSubstitution),
        ("$(<file)", RejectionKind::CommandSubstitution),
        ("${| x; }", RejectionKind::CommandSubstitution),
        ("${ x; }", RejectionKind::CommandSubstitution),
        ("`date`", RejectionKind::BacktickSubstitution),
        ("<(ls)", RejectionKind::ProcessSubstitution),
        (">(tee f)", RejectionKind::ProcessSubstitution),
        ("${x:=d}", RejectionKind::AssignmentExpansion),
        ("${x=d}", RejectionKind::AssignmentExpansion),
        ("${x:?e}", RejectionKind::ErrorExpansion),
        ("${x?e}", RejectionKind::ErrorExpansion),
        ("$((x+=1))", RejectionKind::ArithmeticAssignment),
        ("$((x = 1))", RejectionKind::ArithmeticAssignment),
        ("$((x**=2))", RejectionKind::ArithmeticAssignment),
        ("$((x++))", RejectionKind::ArithmeticUpdate),
        ("$((x--))", RejectionKind::ArithmeticUpdate),
        ("$((++x))", RejectionKind::ArithmeticUpdate),
        ("$((--x))", RejectionKind::ArithmeticUpdate),
        ("${arr[$((i+=1))]}", RejectionKind::ArithmeticAssignment),
        ("${arr[i++]}", RejectionKind::ArithmeticUpdate),
        ("${x:-$(date)}", RejectionKind::CommandSubstitution),
        ("${x:-`date`}", RejectionKind::BacktickSubstitution),
        ("x=1", RejectionKind::AssignmentForm),
        ("arr[1]=v", RejectionKind::AssignmentForm),
        ("arr=(1 2)", RejectionKind::AssignmentForm),
        ("echo $x", RejectionKind::NotOneExpansion),
        ("$x $y", RejectionKind::NotOneExpansion),
        ("$x | cat", RejectionKind::NotOneExpansion),
        ("$x; $y", RejectionKind::NotOneExpansion),
        ("( $x )", RejectionKind::NotOneExpansion),
        ("$x > out", RejectionKind::RedirectForm),
        ("> out", RejectionKind::RedirectForm),
        ("*", RejectionKind::PathnamePattern),
        ("*.rs", RejectionKind::PathnamePattern),
        ("a?c", RejectionKind::PathnamePattern),
        ("{a,b}", RejectionKind::BraceExpansion),
        ("{1..3}", RejectionKind::BraceExpansion),
    ] {
        implicit_rejects(text, kind);
    }
}

/// The diagnostic a rejection leaves is the section 6.3 shape:
/// kind, GNU-shaped message naming the form and the reason.
#[test]
fn rejection_diagnostic_shape_is_section_6_3() {
    let ast = parse("${X:=hacked}");
    let rejection = check_implicit(&ast, "${X:=hacked}").expect_err("must reject");
    let diagnostic = rejection.diagnostic();
    assert_eq!(diagnostic.kind, DiagnosticKind::Expansion);
    assert_eq!(
        diagnostic.message,
        "implicit evaluation skipped: ${X:=hacked}: \
         the expansion assigns to the debuggee's variables"
    );
    assert_eq!(diagnostic.status, None);
}

/// The explicit routing the executor applies: one expansion goes
/// to word expansion at full power (substitutions and assignment
/// expansions included — the whitelist is NOT consulted), and
/// everything else is a command list for the real execution
/// path, which is what makes assignment text the write-back.
#[test]
fn explicit_routing_shape() {
    for text in ["$x", "$(date)", "${x:=y}", "$((x++))", "`date`"] {
        let ast = parse(text);
        assert!(
            single_expansion_word(&ast).is_some(),
            "{text}: one expansion routes to word expansion"
        );
    }
    for text in [
        "x=1",
        "arr[1]=v",
        "echo hi",
        "x=1 y=2",
        "local L=1",
        "set -- a b",
    ] {
        let ast = parse(text);
        assert!(
            single_expansion_word(&ast).is_none(),
            "{text}: command text routes to execution"
        );
    }
    let ast = parse("}");
    assert!(
        parse_failure(&ast).is_some(),
        "an unparseable fragment is a Parse failure, never a command"
    );
}

/// With the parse-time extglob gate off (the default), `@(a|b)`
/// does not parse at all — the failure is a Parse diagnostic,
/// not a whitelist verdict. With the gate on (a debuggee that
/// ran `shopt -s extglob`, whose tokenizer flips the gate the
/// way GNU's parse-execute cadence does), the same text parses
/// as a pattern word and the whitelist refuses it; that half is
/// pinned end to end in `extglob_pattern_is_refused_at_a_stop`.
#[test]
fn extglob_fragment_is_a_parse_face_when_the_gate_is_off() {
    // The gate is process-global (the lexer flips it as scripts
    // run), so hold the debugger tests' lock and set the gate
    // explicitly — the same discipline the parser's own extglob
    // tests use.
    let _span = GateTestSpan::new();
    crate::lexer::set_parse_extended_glob(false);
    let ast = parse("@(a|b)");
    assert!(parse_failure(&ast).is_some());
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

/// A stop loop that plays a fixed queue of evaluation requests
/// at the first breakpoint stop and records every outcome the
/// executor installs, in order.
struct EvalLoop {
    requests: Rc<RefCell<VecDeque<EvaluationRequest>>>,
    outcomes: Rc<RefCell<Vec<EvaluationOutcome>>>,
    bp_command_text: Rc<RefCell<Option<String>>>,
    file: String,
    line: u32,
    issued: usize,
    recorded: usize,
}

impl StopLoop for EvalLoop {
    fn serve(&mut self, session: &mut DebugSession) -> StopLoopOutcome {
        if self.issued > self.recorded {
            let outcome = session
                .last_evaluation()
                .cloned()
                .expect("an evaluation outcome is installed after Evaluate");
            self.outcomes.borrow_mut().push(outcome);
            self.recorded += 1;
        }
        let info = session.last_stop().cloned().expect("serve implies a stop");
        if info.reason == StopReason::Entry {
            session
                .breakpoints_mut()
                .insert_line(self.file.clone(), self.line);
            return StopLoopOutcome::Resume(StepArming::Continue);
        }
        if matches!(info.reason, StopReason::Breakpoint(_)) {
            if self.bp_command_text.borrow().is_none() {
                *self.bp_command_text.borrow_mut() = info.command_text.clone();
            }
            if let Some(request) = self.requests.borrow_mut().pop_front() {
                self.issued += 1;
                return StopLoopOutcome::Evaluate(request);
            }
        }
        StopLoopOutcome::Resume(StepArming::Continue)
    }
}

/// Walks the recorded outcomes in request order, asserting the
/// shape of each.
struct OutcomeCursor<'a> {
    items: &'a [EvaluationOutcome],
    pos: usize,
}

impl OutcomeCursor<'_> {
    fn next(&mut self) -> &EvaluationOutcome {
        let outcome = &self.items[self.pos];
        self.pos += 1;
        outcome
    }

    fn value(&mut self) -> String {
        match &self.next().result {
            EvaluationResult::Value(value) => value.clone(),
            other => panic!("expected a value outcome, got {other:?}"),
        }
    }

    fn rejected(&mut self) -> String {
        match &self.next().result {
            EvaluationResult::Rejected(diagnostic) => {
                assert_eq!(diagnostic.kind, DiagnosticKind::Expansion);
                assert!(
                    diagnostic
                        .message
                        .starts_with("implicit evaluation skipped: "),
                    "diagnostic shape: {}",
                    diagnostic.message
                );
                diagnostic.message.clone()
            }
            other => panic!("expected a rejection, got {other:?}"),
        }
    }

    fn status(&mut self) -> i32 {
        match &self.next().result {
            EvaluationResult::Status(status) => *status,
            other => panic!("expected a status outcome, got {other:?}"),
        }
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-150-row8-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn read(dir: &PathBuf, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).expect("output file written by the script")
}

/// Scenario 7, end to end (design section 8, item 7): at a
/// breakpoint inside a function, implicit evaluations of safe
/// forms return the stopped frame's values; the rejected forms
/// are refused item by item and provably never execute;
/// explicit evaluations run at full power; and write-back —
/// scalar, array element, local, positional — lands through
/// the real assignment path, visible to the debuggee's own next
/// command.
#[test]
fn scenario_7_evaluate_safe_rejected_and_write_back() {
    let _span = GateTestSpan::new();
    let dir = temp_dir("scenario7");
    let dir_text = dir.to_string_lossy().into_owned();
    // Line numbers are load-bearing; the vector index + 1 is the
    // line number. The breakpoint is the echo at line 10.
    let lines = [
        "SCALAR=\"hello world\"",
        "declare -i NUM=7",
        "readonly RO=locked",
        "ARR=([0]=\"a\" [1]=\"bee\" [2]=\"cee\")",
        "declare -A MAP=([k1]=\"v1\")",
        "X=before",
        "probe() {",
        "  local L=\"local val\"",
        "  false | true | false",
        "  echo \"L=$L X=$X ARR1=${ARR[1]} P1=$1\" > \"$OUT/after.txt\"",
        "}",
        "probe p1",
        "echo \"final X=$X NUM=$NUM MAP=${MAP[k1]} RO=$RO\" > \"$OUT/final.txt\"",
    ];
    let script = lines.join("\n") + "\n";
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, &script).expect("write script");
    let script_path_text = script_path.to_string_lossy().into_owned();

    let marker_path = dir.join("marker.txt");
    let marker_text = marker_path.to_string_lossy().into_owned();

    let implicit = |text: &str| EvaluationRequest {
        mode: PowerMode::Implicit,
        text: text.to_string(),
        frame: None,
    };
    let explicit = |text: &str| EvaluationRequest {
        mode: PowerMode::Explicit,
        text: text.to_string(),
        frame: None,
    };
    let mut queue: VecDeque<EvaluationRequest> = VecDeque::new();
    // Implicit: safe forms read the stopped frame.
    queue.push_back(implicit("$SCALAR"));
    queue.push_back(implicit("${ARR[1]}"));
    queue.push_back(implicit("${MAP[k1]}"));
    queue.push_back(implicit("$1"));
    queue.push_back(implicit("$?"));
    queue.push_back(implicit("${PIPESTATUS[0]}"));
    queue.push_back(implicit("${PIPESTATUS[2]}"));
    queue.push_back(implicit("$L"));
    queue.push_back(implicit("$BASH_COMMAND"));
    // Implicit: rejected forms, refused item by item.
    queue.push_back(implicit("${X:=hacked}"));
    queue.push_back(implicit("$X"));
    queue.push_back(implicit(&format!("$(printf x > \"{marker_text}\")")));
    queue.push_back(implicit("${UNSETV:-$(printf y)}"));
    queue.push_back(implicit("`printf z`"));
    // Explicit: full power.
    queue.push_back(explicit("$SCALAR"));
    queue.push_back(explicit("$(printf 'from comsub')"));
    queue.push_back(explicit("${NEWV:=assigned}"));
    queue.push_back(explicit("$NEWV"));
    // Explicit: write-back through the real assignment path.
    queue.push_back(explicit("X=42"));
    queue.push_back(explicit("ARR[1]=changed"));
    queue.push_back(explicit("local L=mutated"));
    queue.push_back(explicit("RO=2"));
    queue.push_back(explicit("PIPESTATUS=(9 9)"));
    queue.push_back(explicit("set -- n1 n2"));
    // Frame addressing and parse failure.
    queue.push_back(EvaluationRequest {
        mode: PowerMode::Explicit,
        text: "$SCALAR".to_string(),
        frame: Some(99),
    });
    queue.push_back(EvaluationRequest {
        mode: PowerMode::Explicit,
        text: "$SCALAR".to_string(),
        frame: Some(0),
    });
    queue.push_back(explicit("}"));
    let total = queue.len();

    let requests = Rc::new(RefCell::new(queue));
    let outcomes: Rc<RefCell<Vec<EvaluationOutcome>>> = Rc::new(RefCell::new(Vec::new()));
    let bp_command_text: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let driver = SessionDriver::new(
        DebugSession::new(),
        Box::new(EvalLoop {
            requests: requests.clone(),
            outcomes: outcomes.clone(),
            bp_command_text: bp_command_text.clone(),
            file: script_path_text.clone(),
            line: 10,
            issued: 0,
            recorded: 0,
        }),
    );
    let mut executor = Executor::new();
    executor.set_env("OUT", &dir_text);
    executor.install_debug_session(driver);
    // The path is interpolated into shell source, so quote it
    // (the row 6 CI lesson: an unquoted native Windows spelling
    // cannot survive the lexer).
    let status = crate::script_driver::run_source(
        &mut executor,
        &format!(". \"{script_path_text}\""),
        false,
    );
    assert_eq!(status, 0, "the debuggee runs to completion");

    let outcomes = outcomes.borrow();
    assert_eq!(outcomes.len(), total, "every request produced an outcome");
    let mut seen = OutcomeCursor {
        items: &outcomes,
        pos: 0,
    };

    // Implicit safe forms read the stopped frame.
    assert_eq!(seen.value(), "hello world", "$SCALAR");
    assert_eq!(seen.value(), "bee", "${{ARR[1]}}");
    assert_eq!(seen.value(), "v1", "${{MAP[k1]}}");
    assert_eq!(seen.value(), "p1", "$1 is probe's argument");
    assert_eq!(seen.value(), "1", "$? is the pipeline's status");
    assert_eq!(seen.value(), "1", "PIPESTATUS[0]");
    assert_eq!(seen.value(), "1", "PIPESTATUS[2]");
    assert_eq!(seen.value(), "local val", "$L is the frame local");
    assert_eq!(
        seen.value(),
        bp_command_text.borrow().clone().expect("stop text"),
        "$BASH_COMMAND at the stop is the about-to-run command"
    );
    // Implicit rejections: refused, and the := provably never ran
    // (the very next read still sees the pre-stop value).
    assert!(
        seen.rejected().contains("${X:=hacked}"),
        "the := rejection names its form"
    );
    assert_eq!(seen.value(), "before", "the rejected := never executed");
    assert!(
        seen.rejected().contains("$(printf x"),
        "the comsub rejection names its form"
    );
    assert!(
        seen.rejected().contains("$(printf y)"),
        "the nested comsub rejection names its form"
    );
    assert!(
        seen.rejected().contains("`printf z`"),
        "the backquote rejection names its form"
    );
    // Explicit full power.
    assert_eq!(seen.value(), "hello world");
    assert_eq!(seen.value(), "from comsub", "explicit comsub executes");
    assert_eq!(seen.value(), "assigned", "explicit := evaluates");
    assert_eq!(seen.value(), "assigned", "the explicit := landed in NEWV");
    // Write-back through the real assignment path.
    assert_eq!(seen.status(), 0, "X=42");
    assert_eq!(seen.status(), 0, "ARR[1]=changed");
    assert_eq!(seen.status(), 0, "local L=mutated");
    // Readonly refuses through the real path (design 7.6: the
    // refusal is the answer — no debugger override): the run's
    // own diagnostic (`RO: readonly variable`) goes to the
    // debuggee's stderr, and the outcome is a Runtime failure
    // carrying status 1. RO is still `locked` in the final state
    // below.
    match &seen.next().result {
        EvaluationResult::Failed(diagnostic) => {
            assert_eq!(diagnostic.kind, DiagnosticKind::Runtime);
            assert_eq!(diagnostic.status, Some(1));
        }
        other => panic!("readonly write-back: unexpected outcome {other:?}"),
    }
    // PIPESTATUS write-back: the real assignment path accepts
    // the compound assignment (status 0) — and the value is
    // inherently transient, because PIPESTATUS is regenerated
    // after every pipeline (exactly GNU's semantics for writing
    // it: the next pipeline's statuses replace it). There is no
    // storage a debugger write could durably land in, and no
    // private channel pretends otherwise.
    assert_eq!(seen.status(), 0, "PIPESTATUS=(9 9) runs the real path");
    assert_eq!(seen.status(), 0, "set -- n1 n2");
    // Frame addressing.
    match &seen.next().result {
        EvaluationResult::Failed(diagnostic) => {
            assert_eq!(diagnostic.kind, DiagnosticKind::Runtime);
            assert!(
                diagnostic.message.contains("no frame 99"),
                "diagnostic: {}",
                diagnostic.message
            );
        }
        other => panic!("frame 99 must fail, got {other:?}"),
    }
    assert_eq!(seen.value(), "hello world", "frame 0 evaluates");
    match &seen.next().result {
        EvaluationResult::Failed(diagnostic) => {
            assert_eq!(diagnostic.kind, DiagnosticKind::Parse);
        }
        other => panic!("an unparseable fragment must fail as Parse, got {other:?}"),
    }
    assert_eq!(seen.pos, total, "no extra outcomes");

    // The rejected command substitution never executed.
    assert!(
        !marker_path.exists(),
        "implicit $(...) was skipped, not executed"
    );
    // The write-backs are visible to the debuggee's own next
    // command (the echo the session was stopped in front of).
    assert_eq!(
        read(&dir, "after.txt").trim_end(),
        "L=mutated X=42 ARR1=changed P1=n1"
    );
    assert_eq!(
        read(&dir, "final.txt").trim_end(),
        "final X=42 NUM=7 MAP=v1 RO=locked"
    );
}

/// The extglob half of the pathname class, end to end: in a
/// debuggee that enabled extglob, an implicit `@(a|b)` parses
/// as a pattern word and the whitelist refuses it as a pathname
/// form — while an ordinary read at the same stop still works.
#[test]
fn extglob_pattern_is_refused_at_a_stop() {
    let _span = GateTestSpan::new();
    let dir = temp_dir("extglob");
    let dir_text = dir.to_string_lossy().into_owned();
    let lines = [
        "shopt -s extglob",
        "V=ok",
        "echo \"done $V\" > \"$OUT/done.txt\"",
        "shopt -u extglob",
    ];
    let script = lines.join("\n") + "\n";
    let script_path = dir.join("extglob.sh");
    std::fs::write(&script_path, &script).expect("write script");
    let script_path_text = script_path.to_string_lossy().into_owned();

    let mut queue: VecDeque<EvaluationRequest> = VecDeque::new();
    queue.push_back(EvaluationRequest {
        mode: PowerMode::Implicit,
        text: "@(a|b)".to_string(),
        frame: None,
    });
    queue.push_back(EvaluationRequest {
        mode: PowerMode::Implicit,
        text: "$V".to_string(),
        frame: None,
    });
    let requests = Rc::new(RefCell::new(queue));
    let outcomes: Rc<RefCell<Vec<EvaluationOutcome>>> = Rc::new(RefCell::new(Vec::new()));
    let bp_command_text: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let driver = SessionDriver::new(
        DebugSession::new(),
        Box::new(EvalLoop {
            requests: requests.clone(),
            outcomes: outcomes.clone(),
            bp_command_text: bp_command_text.clone(),
            file: script_path_text.clone(),
            line: 3,
            issued: 0,
            recorded: 0,
        }),
    );
    let mut executor = Executor::new();
    executor.set_env("OUT", &dir_text);
    executor.install_debug_session(driver);
    let status = crate::script_driver::run_source(
        &mut executor,
        &format!(". \"{script_path_text}\""),
        false,
    );
    assert_eq!(status, 0);
    let outcomes = outcomes.borrow();
    assert_eq!(outcomes.len(), 2);
    match &outcomes[0].result {
        EvaluationResult::Rejected(diagnostic) => {
            assert!(
                diagnostic.message.contains("pathname expansion"),
                "extglob refuses as a pathname form: {}",
                diagnostic.message
            );
        }
        other => panic!("@(a|b) must be refused at the stop, got {other:?}"),
    }
    match &outcomes[1].result {
        EvaluationResult::Value(value) => assert_eq!(value, "ok"),
        other => panic!("$V must still evaluate, got {other:?}"),
    }
    assert_eq!(read(&dir, "done.txt").trim_end(), "done ok");
}
