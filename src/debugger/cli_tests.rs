//! CLI frontend acceptance tests (PLAN row 11; design section 8
//! M1 set, section 14 Q5 (a)).
//!
//! The command stream is the injected scripted stream, so a
//! whole human-terminal session runs deterministically in the
//! test: breakpoints, stepping, inspection, evaluation,
//! write-back, lifecycle. The transcript is asserted after
//! normalizing the one environment-shaped string in it (the
//! temp program path) — everything else is the frontend's own
//! bytes.
//!
//! The final test is the PLAN section 3 discipline made
//! mechanical: the same script, driven through the DAP adapter
//! and through this CLI, produces the same stop sequence.

use std::cell::RefCell;
use std::io::{BufReader, Cursor, Write};
use std::rc::Rc;

use super::cli::{
    host_name_from, run_debug_cli_with_stream, run_debug_cli_with_stream_as, DebugCliPreset,
};
use super::gate::SESSION_ACTIVE;

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

#[derive(Clone)]
struct SharedOut(Rc<RefCell<Vec<u8>>>);

impl Write for SharedOut {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Write `source` to a fresh temp file; return (dir, program path).
fn program_file(name: &str, source: &str) -> (std::path::PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("rubash-row11-cli-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(format!("{name}.sh"));
    std::fs::write(&path, source).expect("write program");
    let canonical = std::fs::canonicalize(&path).expect("canonicalize");
    // The identity form the session core itself uses: the
    // canonicalizer's extended-length prefix stripped, so the
    // string the test normalizes by is the string the frontend
    // prints (a mismatch here fails on Windows only, with the
    // behavior itself byte-identical).
    let text = canonical.to_string_lossy().into_owned();
    let identity = if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        text
    };
    (dir, identity)
}

/// Run one scripted CLI session; return (status, transcript with
/// the program path normalized to `<PROG>`).
fn run_cli(program: &str, commands: &str) -> (i32, String) {
    run_cli_with_startup(program, None, commands)
}

/// Run one scripted CLI session with a host-supplied startup
/// file (niubash#692: the `--rcfile` channel); the startup
/// path is normalized to `<RC>` in the transcript as well.
fn run_cli_with_startup(program: &str, startup: Option<&str>, commands: &str) -> (i32, String) {
    let out = SharedOut(Rc::new(RefCell::new(Vec::new())));
    let status = run_debug_cli_with_stream(
        DebugCliPreset {
            program: Some(program.to_string()),
            args: Vec::new(),
            startup_file: startup.map(str::to_string),
        },
        Box::new(BufReader::new(Cursor::new(commands.as_bytes().to_vec()))),
        Box::new(out.clone()),
    );
    let bytes = out.0.borrow().clone();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let text = text.replace(program, "<PROG>");
    let text = match startup {
        Some(path) => text.replace(path, "<RC>"),
        None => text,
    };
    (status, text)
}

/// Write `source` to a named file inside an existing temp dir;
/// return its identity-form path (the canonicalization
/// `program_file` applies).
fn sibling_file(dir: &std::path::Path, name: &str, source: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, source).expect("write sibling file");
    let canonical = std::fs::canonicalize(&path).expect("canonicalize");
    let text = canonical.to_string_lossy().into_owned();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        text
    }
}

const SESSION_SCRIPT: &str = "\
X=41
helper() {
  Y=2
  echo \"in helper\"
  return 7
}
echo start
helper
echo \"after $?\"
Z=$((X + 1))
echo \"z=$Z\"
";

#[test]
fn a_full_session_transcript() {
    let _span = GateTestSpan::new();
    let (dir, program) = program_file("session", SESSION_SCRIPT);
    let commands = "\
break 7
continue
step
step
next
next
print Y
finish
info state
set var X = 99
print X
step+
continue
";
    let (status, transcript) = run_cli(&program, commands);
    assert_eq!(status, 0, "the session runs to a clean exit: {transcript}");
    // The stops, in order, as the engine's seam model produces
    // them (the same sequence the DAP fixtures pin): entry, the
    // line-7 breakpoint, the step to the call line, the step
    // into the function (the entry seam stamps the definition
    // line), next to the body's first line, next over Y=2,
    // finish back at the call seam, step+ to the next line.
    for needle in [
        "Stopped (entry) — <PROG>",
        "breakpoint 1 at <PROG>:7: verified",
        "Stopped (breakpoint 1) at <PROG>:7: echo start",
        "Stopped (step) at <PROG>:8: helper",
        "Stopped (step) at <PROG>:2: helper",
        "Stopped (step) at <PROG>:3: Y=2",
        "Stopped (step) at <PROG>:4: echo \"in helper\"",
        "Y = 2",
        "Stopped (step) at <PROG>:8\n",
        "bash_command: return 7",
        "X = 99",
        "Stopped (step) at <PROG>:9: echo \"after $?\"",
    ] {
        assert!(
            transcript.contains(needle),
            "transcript must contain {needle:?}:\n{transcript}"
        );
    }
    // Order matters: the needles above must appear in sequence.
    let mut at = 0;
    for needle in [
        "Stopped (entry)",
        "Stopped (breakpoint 1) at <PROG>:7",
        "Stopped (step) at <PROG>:8",
        "Stopped (step) at <PROG>:3",
        "Stopped (step) at <PROG>:4",
        "Stopped (step) at <PROG>:9",
    ] {
        let found = transcript[at..]
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} missing or out of order:\n{transcript}"));
        at += found + needle.len();
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn kill_ends_the_session_without_the_exit_path() {
    let _span = GateTestSpan::new();
    let (dir, program) = program_file("kill", SESSION_SCRIPT);
    let (status, transcript) = run_cli(&program, "break 7\ncontinue\nkill\n");
    assert_eq!(status, 128 + 15, "killed status: {transcript}");
    assert!(transcript.contains("killed"), "{transcript}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn quit_at_the_entry_stop_never_runs_the_program() {
    let _span = GateTestSpan::new();
    let (dir, program) = program_file("quit", SESSION_SCRIPT);
    let (status, transcript) = run_cli(&program, "quit\n");
    assert_eq!(status, 0, "{transcript}");
    assert!(transcript.contains("Stopped (entry)"), "{transcript}");
    assert!(
        !transcript.contains("Stopped (breakpoint"),
        "nothing ran: {transcript}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn step_minus_leaves_the_function_at_the_next_caller_seam() {
    let _span = GateTestSpan::new();
    let (dir, program) = program_file("stepminus", SESSION_SCRIPT);
    let (status, transcript) = run_cli(&program, "break helper\ncontinue\nstep-\nquit\n");
    // The quit preserves the debuggee's current status through
    // the exit path (design 9.3): after `helper` returned, that
    // status is its 7.
    assert_eq!(status, 7, "{transcript}");
    assert!(
        transcript.contains("Stopped (function breakpoint 1) at <PROG>:2"),
        "the entry seam stamps the definition line: {transcript}"
    );
    assert!(
        transcript.contains("Stopped (step) at <PROG>:9"),
        "step- stops at the first seam back in the caller: {transcript}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn exception_filters_are_listed_and_toggled_from_the_cli() {
    let _span = GateTestSpan::new();
    let (dir, program) = program_file("exceptions", SESSION_SCRIPT);
    let commands =
        "exceptions\nexceptions err-trap on\ninfo exceptions\nexceptions bogus on\nquit\n";
    let (status, transcript) = run_cli(&program, commands);
    assert_eq!(status, 0, "{transcript}");
    assert!(
        transcript.contains("exception err-trap: off"),
        "{transcript}"
    );
    assert!(
        transcript.contains("exception err-trap: on"),
        "{transcript}"
    );
    assert!(
        transcript.contains("exceptions: unknown filter(s): bogus"),
        "{transcript}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_vscode_extension_manifest_is_zero_js_and_points_at_the_dap_entry() {
    // WP-16 validation pin: the declarative artifact parses,
    // carries no JavaScript entry point, and resolves its
    // adapter to the spelled-out `--debug --dap` host form.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("editors")
        .join("vscode")
        .join("package.json");
    let text = std::fs::read_to_string(&path).expect("the extension manifest exists");
    let doc = super::json::Json::parse(&text).expect("the manifest parses");
    assert!(
        doc.get("main").is_none(),
        "a zero-JS extension has no activation entry point"
    );
    let debuggers = doc
        .get("contributes")
        .and_then(|c| c.get("debuggers"))
        .and_then(super::json::Json::as_array)
        .expect("a debugger contribution");
    let debugger = &debuggers[0];
    assert_eq!(
        debugger.get("type").and_then(super::json::Json::as_str),
        Some("rubash")
    );
    assert_eq!(
        debugger.get("program").and_then(super::json::Json::as_str),
        Some("rubash")
    );
    let args: Vec<&str> = debugger
        .get("args")
        .and_then(super::json::Json::as_array)
        .expect("adapter args")
        .iter()
        .filter_map(super::json::Json::as_str)
        .collect();
    assert_eq!(args, vec!["--debug", "--dap"]);
}

// ------------------------------------------------------------------
// CLI <-> DAP stop-sequence consistency (PLAN section 3)
// ------------------------------------------------------------------

fn dap_frame(requests: &mut Vec<String>, seq: i64, command: &str, arguments: &str) {
    let body = format!(
        "{{\"seq\":{seq},\"type\":\"request\",\"command\":\"{command}\",\"arguments\":{arguments}}}"
    );
    requests.push(format!("Content-Length: {}\r\n\r\n{}", body.len(), body));
}

/// Drive the DAP adapter over a fully scripted request stream
/// and return the raw protocol output.
fn run_dap_scripted(program: &str, requests: &[String]) -> String {
    let _ = program;
    let input = requests.concat();
    let out = SharedOut(Rc::new(RefCell::new(Vec::new())));
    let status = super::dap::run_dap_server(
        Cursor::new(input.into_bytes()),
        out.clone(),
        super::dap::DapLaunchPreset::default(),
    );
    assert_eq!(status, 0, "the DAP run exits cleanly");
    let bytes = out.0.borrow().clone();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The `line` of every stackTrace answer's top frame, in order.
fn dap_top_lines(protocol: &str) -> Vec<i64> {
    let mut lines = Vec::new();
    for chunk in protocol.split("Content-Length:").skip(1) {
        let Some((_, body)) = chunk.split_once("\r\n\r\n") else {
            continue;
        };
        let Ok(doc) = super::json::Json::parse(body.trim()) else {
            continue;
        };
        if doc.get("command").and_then(super::json::Json::as_str) != Some("stackTrace") {
            continue;
        }
        let frames = doc
            .get("body")
            .and_then(|b| b.get("stackFrames"))
            .and_then(super::json::Json::as_array)
            .expect("stackTrace body");
        if let Some(top) = frames.first() {
            lines.push(
                top.get("line")
                    .and_then(super::json::Json::as_i64)
                    .expect("top frame line"),
            );
        }
    }
    lines
}

#[test]
fn cli_and_dap_stop_at_the_same_lines_for_the_same_script() {
    let _span = GateTestSpan::new();
    let script = "A=1\necho one\nB=2\necho two\n";
    let (dir, program) = program_file("consistency", script);

    // CLI: four steps from the entry stop (the first lands on
    // the sourced unit's entry seam, which carries no location;
    // the located stops follow it).
    let (_, transcript) = run_cli(&program, "step\nstep\nstep\nstep\ncontinue\n");
    let cli_lines: Vec<u32> = transcript
        .lines()
        .filter_map(|line| {
            let rest = line.split("Stopped (step) at <PROG>:").nth(1)?;
            rest.split(':').next()?.parse().ok()
        })
        .collect();

    // DAP: the same three steps, asking for the stack at each
    // stop.
    let mut requests: Vec<String> = Vec::new();
    dap_frame(&mut requests, 1, "initialize", "{}");
    dap_frame(
        &mut requests,
        2,
        "launch",
        &format!(
            "{{\"program\":\"{}\",\"stopOnEntry\":true}}",
            program.replace('\\', "\\\\").replace('"', "\\\"")
        ),
    );
    dap_frame(&mut requests, 3, "configurationDone", "{}");
    let mut seq = 4;
    for _ in 0..4 {
        dap_frame(&mut requests, seq, "stepIn", "{\"threadId\":1}");
        seq += 1;
        dap_frame(
            &mut requests,
            seq,
            "stackTrace",
            "{\"threadId\":1,\"startFrame\":0,\"levels\":1}",
        );
        seq += 1;
    }
    dap_frame(&mut requests, seq, "continue", "{\"threadId\":1}");
    seq += 1;
    dap_frame(&mut requests, seq, "disconnect", "{}");
    let protocol = run_dap_scripted(&program, &requests);
    let dap_lines = dap_top_lines(&protocol);

    assert_eq!(cli_lines, vec![1, 2, 3], "CLI step sequence: {transcript}");
    assert_eq!(
        dap_lines,
        cli_lines
            .iter()
            .map(|line| i64::from(*line))
            .collect::<Vec<_>>(),
        "DAP step sequence must equal the CLI's: {protocol}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------
// PLAN row 12: the DE-06 exception-site producers, CLI face.
// A selected filter reports the site and parks the session
// BEFORE the pending engine action; resuming lets the action
// proceed unchanged. An unselected filter produces nothing.
// ---------------------------------------------------------------------

const ERR_TRAP_SCRIPT: &str = "\
T=none
trap 'T=hit' ERR
X=5
false
echo after
";

#[test]
fn err_trap_filter_parks_before_the_action_and_the_action_still_runs() {
    let _span = GateTestSpan::new();
    let (dir, program) = program_file("errsite", ERR_TRAP_SCRIPT);
    let commands = "\
exceptions err-trap on
break 5
continue
print T
print X
continue
print T
continue
";
    let (status, transcript) = run_cli(&program, commands);
    assert_eq!(status, 0, "the session runs to a clean exit: {transcript}");
    for needle in [
        // The site is reported, and the session parks at the
        // failing command's line.
        "exception site: ERR trap (status 1) at <PROG>:4",
        "Stopped (exception (ERR trap)) at <PROG>:4",
        // Parked BEFORE the trap action: T is untouched, the
        // failing command's own effects are all visible.
        "T = none",
        "X = 5",
        // After the resume the action ran, then the line-5
        // breakpoint stops the run as usual.
        "Stopped (breakpoint 1) at <PROG>:5: echo after",
        "T = hit",
    ] {
        assert!(
            transcript.contains(needle),
            "transcript must contain {needle:?}:\n{transcript}"
        );
    }
    // The park precedes the action: "T = none" is read at the
    // exception stop, "T = hit" only after the resume.
    let parked = transcript.find("T = none").expect("parked read");
    let acted = transcript.find("T = hit").expect("post-resume read");
    assert!(
        parked < acted,
        "the action ran only after the resume:\n{transcript}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

const ERREXIT_SCRIPT: &str = "\
set -e
X=7
false
echo unreachable
";

#[test]
fn errexit_filter_parks_before_the_exit_and_the_run_still_exits() {
    let _span = GateTestSpan::new();
    let (dir, program) = program_file("errexitsite", ERREXIT_SCRIPT);
    let commands = "\
exceptions set -e on
continue
print X
continue
";
    let (status, transcript) = run_cli(&program, commands);
    assert_eq!(status, 1, "the errexit exit still happens: {transcript}");
    for needle in [
        "exception site: errexit (status 1) at <PROG>:3",
        "Stopped (exception (errexit)) at <PROG>:3",
        // The corpse is inspectable at the park.
        "X = 7",
    ] {
        assert!(
            transcript.contains(needle),
            "transcript must contain {needle:?}:\n{transcript}"
        );
    }
    // One death, one report: the exit unwinds through the
    // frame of the `.` that sourced this program; that frame
    // must not re-report the site the origin frame parked.
    assert_eq!(
        transcript.matches("exception site: errexit").count(),
        1,
        "exactly one exception note:\n{transcript}"
    );
    assert_eq!(
        transcript.matches("Stopped (exception").count(),
        1,
        "exactly one exception stop:\n{transcript}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_unselected_filter_neither_reports_nor_parks() {
    let _span = GateTestSpan::new();
    let (dir, program) = program_file("nofilter", ERREXIT_SCRIPT);
    let (status, transcript) = run_cli(&program, "continue\n");
    assert_eq!(status, 1, "the run still exits under set -e: {transcript}");
    assert!(
        !transcript.contains("exception site:"),
        "no site is reported without the filter:\n{transcript}"
    );
    assert!(
        !transcript.contains("Stopped (exception"),
        "no exception stop without the filter:\n{transcript}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

const PARSE_SCRIPT: &str = "\
echo before
echo )
echo after
";

#[test]
fn parse_error_filter_parks_and_the_run_still_exits_2() {
    let _span = GateTestSpan::new();
    let (dir, program) = program_file("parsesite", PARSE_SCRIPT);
    let commands = "\
exceptions parse-error on
continue
continue
";
    let (status, transcript) = run_cli(&program, commands);
    assert_eq!(
        status, 2,
        "the parse error still ends the run: {transcript}"
    );
    for needle in [
        "exception site: parse error (status 2) at <PROG>:2",
        "Stopped (exception (parse error)) at <PROG>:2",
    ] {
        assert!(
            transcript.contains(needle),
            "transcript must contain {needle:?}:\n{transcript}"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------
// rubash#1962: breakpoint-condition defects (RC audit phase 2,
// cell group ②). Line 2 of ISSUE1962_SCRIPT is inside the loop
// (three consults), line 4 is after it (one consult). The three
// confirmed cells get pins asserting the debugger-side report,
// the consult line in the quoted diagnostic, the debuggee's
// survival, and the session status; the three overturned cells
// get parity pins so the fix cannot regress them.

const ISSUE1962_SCRIPT: &str = "\
for i in 1 2 3; do
  echo \"iter $i\"
done
echo after-loop
";

#[test]
fn issue1962_condition_break_in_loop_reports_once_and_debuggee_survives() {
    let _span = GateTestSpan::new();
    let (dir, program) = program_file("condbreak", ISSUE1962_SCRIPT);
    let (status, transcript) = run_cli(&program, "break 2 if break\nbreak 4\ncontinue\ncontinue\n");
    assert_eq!(status, 0, "{transcript}");
    // The in-loop `break` unwinds inside the evaluation: a failed
    // condition (design 5.3) — reported exactly once for the
    // three consults, not folded silently and not per hit.
    assert_eq!(
        transcript
            .matches("breakpoint 1: condition 'break' failed at <PROG>:2, treated as false")
            .count(),
        1,
        "{transcript}"
    );
    // The debuggee was not disturbed: it reaches the line-4
    // breakpoint after the loop.
    assert!(
        transcript.contains("Stopped (breakpoint 2) at <PROG>:4"),
        "{transcript}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn issue1962_condition_numeric_error_reports_once_at_the_consult_line() {
    let _span = GateTestSpan::new();
    let (dir, program) = program_file("condnum", ISSUE1962_SCRIPT);
    let (status, transcript) = run_cli(&program, "break 2 if break 2x\ncontinue\n");
    assert_eq!(status, 0, "{transcript}");
    assert_eq!(
        transcript
            .matches("breakpoint 1: condition 'break 2x' failed at <PROG>:2, treated as false")
            .count(),
        1,
        "{transcript}"
    );
    // The engine diagnostic appears exactly once — inside the
    // report, quoted — and carries the consult line (the
    // debuggee's line 2), not the condition text's own line 1.
    assert_eq!(
        transcript.matches("numeric argument required").count(),
        1,
        "{transcript}"
    );
    assert!(
        transcript.contains("  <PROG>: line 2: break: 2x: numeric argument required"),
        "{transcript}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn issue1962_condition_too_many_arguments_cannot_kill_the_debuggee() {
    let _span = GateTestSpan::new();
    let (dir, program) = program_file("condtoomany", ISSUE1962_SCRIPT);
    let (status, transcript) = run_cli(
        &program,
        "break 2 if break 1 2\nbreak 4\ncontinue\ncontinue\n",
    );
    assert_eq!(status, 0, "{transcript}");
    assert_eq!(
        transcript
            .matches("breakpoint 1: condition 'break 1 2' failed at <PROG>:2, treated as false")
            .count(),
        1,
        "{transcript}"
    );
    assert!(
        transcript.contains("  <PROG>: line 2: break: too many arguments"),
        "{transcript}"
    );
    // The too-many-arguments DISCARD the condition armed died
    // with the evaluation: the debuggee lives to stop at line 4.
    assert!(
        transcript.contains("Stopped (breakpoint 2) at <PROG>:4"),
        "{transcript}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn issue1962_condition_break_outside_the_loop_commits_its_stop() {
    let _span = GateTestSpan::new();
    let (dir, program) = program_file("condoutside", ISSUE1962_SCRIPT);
    let (status, transcript) = run_cli(&program, "break 4 if break\ncontinue\ncontinue\n");
    assert_eq!(status, 0, "{transcript}");
    // An out-of-loop `break` is status 0 in GNU, so the verdict
    // is true and the stop commits — and a true verdict is not
    // reported as a failure.
    assert!(
        transcript.contains("Stopped (breakpoint 1) at <PROG>:4"),
        "{transcript}"
    );
    assert!(
        !transcript.contains("condition 'break' failed"),
        "{transcript}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn issue1962_condition_true_in_loop_stops_exactly_once() {
    let _span = GateTestSpan::new();
    let (dir, program) = program_file("condtruein", ISSUE1962_SCRIPT);
    let (status, transcript) = run_cli(&program, "break 2 if [[ $i == 2 ]]\ncontinue\ncontinue\n");
    assert_eq!(status, 0, "{transcript}");
    assert_eq!(
        transcript.matches("Stopped (breakpoint 1)").count(),
        1,
        "{transcript}"
    );
    assert!(!transcript.contains("failed"), "{transcript}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn issue1962_condition_true_outside_stops_exactly_once() {
    let _span = GateTestSpan::new();
    let (dir, program) = program_file("condtrueout", ISSUE1962_SCRIPT);
    let (status, transcript) = run_cli(&program, "break 4 if [[ $i == 3 ]]\ncontinue\ncontinue\n");
    assert_eq!(status, 0, "{transcript}");
    assert_eq!(
        transcript.matches("Stopped (breakpoint 1)").count(),
        1,
        "{transcript}"
    );
    assert!(!transcript.contains("failed"), "{transcript}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn issue1962_condition_command_substitution_runs_once_per_consult() {
    let _span = GateTestSpan::new();
    let (dir, program) = program_file("condcomsub", ISSUE1962_SCRIPT);
    let ticks = dir.join("ticks");
    // The path is embedded into shell text (the condition), so
    // it must carry the shell-facing spelling: a native Windows
    // path's backslashes are the lexer's escape character and
    // would be eaten by quote removal before the redirect ever
    // opens (the ticks then land in a separator-stripped namesake
    // file, one per consult — the count semantics are not what
    // fails on Windows, the spelling is). Same convention as the
    // other shell-text path embeddings in the test tree.
    let ticks = ticks.to_string_lossy().replace('\\', "/");
    // A false condition whose substitution appends one tick per
    // evaluation: three consults, exactly three ticks, no stop.
    let commands = format!("break 2 if [[ $(echo tick >> {ticks}; echo $i) == 99 ]]\ncontinue\n");
    let (status, transcript) = run_cli(&program, &commands);
    assert_eq!(status, 0, "{transcript}");
    assert!(!transcript.contains("Stopped (breakpoint"), "{transcript}");
    let recorded = std::fs::read_to_string(&ticks).expect("ticks recorded");
    assert_eq!(
        recorded.lines().count(),
        3,
        "one tick per consult: {recorded}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn issue1962_source_command_file_drives_the_session() {
    let _span = GateTestSpan::new();
    let (dir, program) = program_file("sourcecmds", ISSUE1962_SCRIPT);
    let cmds = dir.join("session.cmds");
    std::fs::write(&cmds, "break 4\ncontinue\n").expect("write command file");
    let commands = format!("source {}\ncontinue\n", cmds.to_string_lossy());
    let (status, transcript) = run_cli(&program, &commands);
    assert_eq!(status, 0, "{transcript}");
    assert!(
        transcript.contains("Stopped (breakpoint 1) at <PROG>:4"),
        "{transcript}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn issue1962_source_nested_file_preempts_the_sourcer() {
    let _span = GateTestSpan::new();
    let (dir, program) = program_file("sourcenested", ISSUE1962_SCRIPT);
    let inner = dir.join("inner.cmds");
    std::fs::write(&inner, "continue\n").expect("write inner file");
    let outer = dir.join("outer.cmds");
    std::fs::write(
        &outer,
        format!("source {}\nbreak 4\n", inner.to_string_lossy()),
    )
    .expect("write outer file");
    // The nested file's `continue` runs before the outer file's
    // remaining `break 4` line: the resume spends the serve with
    // no breakpoint set, so nothing ever stops.
    let commands = format!("source {}\n", outer.to_string_lossy());
    let (status, transcript) = run_cli(&program, &commands);
    assert_eq!(status, 0, "{transcript}");
    assert!(!transcript.contains("Stopped (breakpoint"), "{transcript}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn issue1962_source_missing_file_reports_and_the_session_lives() {
    let _span = GateTestSpan::new();
    let (dir, program) = program_file("sourcemissing", ISSUE1962_SCRIPT);
    let (status, transcript) = run_cli(
        &program,
        "source /nonexistent-1962/nope.cmds\nbreak 4\ncontinue\ncontinue\n",
    );
    assert_eq!(status, 0, "{transcript}");
    assert!(
        transcript.contains("source: cannot read '/nonexistent-1962/nope.cmds'"),
        "{transcript}"
    );
    assert!(
        transcript.contains("Stopped (breakpoint 1) at <PROG>:4"),
        "{transcript}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------
// niubash#692: the host-supplied startup file (the niubash
// host's `--rcfile` channel) is sourced into the session
// before the program — its definitions are live for the
// debuggee, a function-valued DEBUG trap it installs coexists
// with breakpoints (the niubash#681 shape, whose repro was
// written against this very loading face), and an `exit`
// inside it ends the run before the program starts.

#[test]
fn startup_file_definitions_are_live_for_the_debuggee() {
    let _span = GateTestSpan::new();
    let (dir, program) = program_file(
        "rcdefs",
        "\
rc_helper
echo \"after helper\"
",
    );
    let rc = sibling_file(
        &dir,
        "rcdefs-rc.sh",
        "\
RC_MARKER=loaded-from-rc
rc_helper() {
  HELPER_RAN=yes
  return 3
}
",
    );
    let (status, transcript) = run_cli_with_startup(
        &program,
        Some(&rc),
        "break 2\ncontinue\nprint RC_MARKER\nprint HELPER_RAN\ncontinue\n",
    );
    assert_eq!(status, 0, "the session runs to a clean exit: {transcript}");
    assert!(
        transcript.contains("Stopped (breakpoint 1) at <PROG>:2: echo \"after helper\""),
        "the program breakpoint must stop after the rc-defined call: {transcript}"
    );
    assert!(
        transcript.contains("RC_MARKER = loaded-from-rc"),
        "the rc's variable must be set in the session: {transcript}"
    );
    assert!(
        transcript.contains("HELPER_RAN = yes"),
        "the rc-defined function must have run: {transcript}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn startup_file_debug_trap_function_coexists_with_breakpoints() {
    let _span = GateTestSpan::new();
    let (dir, program) = program_file(
        "rctrap",
        "\
A=1
B=2
echo done
",
    );
    let rc = sibling_file(
        &dir,
        "rctrap-rc.sh",
        "\
rc_trap_handler() {
  TRAP_HITS=$(( ${TRAP_HITS:-0} + 1 ))
}
trap 'rc_trap_handler' DEBUG
",
    );
    let (status, transcript) = run_cli_with_startup(
        &program,
        Some(&rc),
        "break 2\ncontinue\nprint TRAP_HITS\ncontinue\n",
    );
    assert_eq!(status, 0, "the session runs to a clean exit: {transcript}");
    assert!(
        transcript.contains("Stopped (breakpoint 1) at <PROG>:2: B=2"),
        "a function-valued DEBUG trap from the rc must not kill the breakpoint \
         (niubash#681): {transcript}"
    );
    assert!(
        transcript.contains("TRAP_HITS = "),
        "the rc's DEBUG trap must be live during the program: {transcript}"
    );
    assert!(
        !transcript.contains("TRAP_HITS = 0"),
        "the rc's DEBUG trap must have fired before the line-2 stop: {transcript}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn startup_file_exit_ends_the_run_before_the_program() {
    let _span = GateTestSpan::new();
    let (dir, program) = program_file("rcexit", "echo never\n");
    let rc = sibling_file(&dir, "rcexit-rc.sh", "exit 7\n");
    let (status, transcript) = run_cli_with_startup(&program, Some(&rc), "continue\n");
    assert_eq!(
        status, 7,
        "an exit in the startup file ends the run with its status: {transcript}"
    );
    assert!(
        transcript.contains("Stopped (entry) — <PROG>"),
        "the entry stop still comes first: {transcript}"
    );
    assert!(
        !transcript.contains("<PROG>:1"),
        "the program must not start after the startup file exits: {transcript}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn startup_file_missing_does_not_abort_the_session() {
    let _span = GateTestSpan::new();
    let (dir, program) = program_file("rcmissing", "echo reached\n");
    let missing = sibling_file(&dir, "rcmissing-rc.sh", "exit 99\n");
    std::fs::remove_file(&missing).expect("remove the startup file");
    // The source builtin reports the missing file on the
    // debuggee's stderr (its own pinned diagnostic); the point
    // here is the session still runs the program — the failure
    // is visible, never a silent drop and never a refusal.
    let (status, transcript) =
        run_cli_with_startup(&program, Some(&missing), "break 1\ncontinue\ncontinue\n");
    assert_eq!(
        status, 0,
        "the program still runs to completion: {transcript}"
    );
    assert!(
        transcript.contains("Stopped (breakpoint 1) at <PROG>:1: echo reached"),
        "the program breakpoint must still stop: {transcript}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The host product word is part of the frontend's presentation:
/// niubash passes `niu`, so the prompt and every refusal carry
/// `niu`, never the engine's `rubash` (owner report 2026-10-31:
/// "the REPL still says rubash").
#[test]
fn host_name_drives_the_prompt_and_diagnostics() {
    // The derivation: the niubash executable presents `niu`,
    // the engine's own binary `rubash`, the GNU faces their
    // invocation name; test binaries and unknown stems fall
    // back to the engine word so transcripts stay stable.
    assert_eq!(host_name_from(Some("niu")), "niu");
    assert_eq!(host_name_from(Some("-niu")), "niu");
    assert_eq!(host_name_from(Some("Niubash")), "niubash");
    assert_eq!(host_name_from(Some("rubash")), "rubash");
    assert_eq!(host_name_from(Some("bash")), "bash");
    assert_eq!(host_name_from(Some("-sh")), "sh");
    assert_eq!(host_name_from(None), "rubash");
    assert_eq!(host_name_from(Some("cargo-test-abcd")), "rubash");
    let _span = GateTestSpan::new();
    let (dir, program) = program_file("hostname", "echo hi\n");
    let out = SharedOut(Rc::new(RefCell::new(Vec::new())));
    let status = run_debug_cli_with_stream_as(
        DebugCliPreset {
            program: Some(program),
            ..Default::default()
        },
        Box::new(BufReader::new(Cursor::new(b"quit\n".to_vec()))),
        Box::new(out.clone()),
        "niu",
    );
    assert_eq!(status, 0);
    let transcript = String::from_utf8_lossy(&out.0.borrow()).into_owned();
    assert!(
        transcript.contains("(niu-dbg) "),
        "prompt must carry the host word: {transcript}"
    );
    assert!(
        !transcript.contains("rubash-dbg"),
        "the engine word must not leak into the host prompt: {transcript}"
    );

    let out = SharedOut(Rc::new(RefCell::new(Vec::new())));
    let status = run_debug_cli_with_stream_as(
        DebugCliPreset::default(),
        Box::new(BufReader::new(Cursor::new(Vec::new()))),
        Box::new(out.clone()),
        "niu",
    );
    assert_eq!(status, 2);
    let text = String::from_utf8_lossy(&out.0.borrow()).into_owned();
    assert!(
        text.contains("niu: --debug: a script operand is required"),
        "diagnostic must carry the host word: {text}"
    );
    std::fs::remove_dir_all(&dir).ok();
}
