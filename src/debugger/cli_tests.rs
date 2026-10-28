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

use super::cli::{run_debug_cli_with_stream, DebugCliPreset};
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
    let out = SharedOut(Rc::new(RefCell::new(Vec::new())));
    let status = run_debug_cli_with_stream(
        DebugCliPreset {
            program: Some(program.to_string()),
            args: Vec::new(),
        },
        Box::new(BufReader::new(Cursor::new(commands.as_bytes().to_vec()))),
        Box::new(out.clone()),
    );
    let bytes = out.0.borrow().clone();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    (status, text.replace(program, "<PROG>"))
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
