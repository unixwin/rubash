//! rubash#2960 (Stage-2 ④ Phase 3 E1 residual) — `set -t`
//! (`onecmd`, flags.c:184 `just_one_command`) was never honored by
//! rubash's readers: the flag could be set and listed, but no reader
//! ever consulted it.
//!
//! GNU mechanism: eval.c:206-207 — reader_loop takes EOF right after
//! a command completes when just_one_command stands. The check is a
//! property of the PRINCIPAL reader only (script file, stdin,
//! interactive input): the unit that SETS the flag still completes,
//! and the stop lands between read units, so a whole `;` list on one
//! line runs to completion first. parse_and_execute bodies — the
//! `-c` string (shell.c run_one_command), eval strings, sourced
//! files — never take the check themselves (eval.c's loop is not
//! re-entered for them); but a flag they set is live state, so the
//! principal reader stops after the unit that sourced/evaluated
//! them. A failed unit keeps its status: the run ends with the last
//! executed unit's status, exactly like natural EOF.
//!
//! Pre-fix rubash: in-script `set -t` changed nothing (every unit
//! ran), and the invocation door was worse — `-t` was not in the
//! binary's CLI flag table at all, so `rubash -t script` treated
//! `-t` as the script name and exited 127.
//!
//! Every expectation below was measured against GNU bash 5.3.20.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

struct Run {
    status: i32,
    stdout: String,
    stderr: String,
    dir: PathBuf,
}

fn case_dir(tag: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("issue2960_{tag}_{}_{}", std::process::id(), seq));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    dir
}

fn finish(dir: PathBuf, output: std::process::Output) -> Run {
    Run {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        dir,
    }
}

/// Run `case.sh` (plus an optional `child.sh` beside it) with extra
/// rubash arguments in front of the script name.
fn run_script(body: &str, child: Option<&str>, args: &[&str]) -> Run {
    let dir = case_dir("script");
    std::fs::write(dir.join("case.sh"), body).expect("write script");
    if let Some(child) = child {
        std::fs::write(dir.join("child.sh"), child).expect("write child script");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(args)
        .arg("case.sh")
        .current_dir(&dir)
        .env("LC_ALL", "C")
        .output()
        .expect("run rubash");
    finish(dir, output)
}

fn run_command_string(script: &str) -> Run {
    let dir = case_dir("cstr");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(&dir)
        .env("LC_ALL", "C")
        .output()
        .expect("run rubash");
    finish(dir, output)
}

fn run_stdin(args: &[&str], stdin_text: &str) -> Run {
    let dir = case_dir("stdin");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    let mut child = cmd
        .args(args)
        .current_dir(&dir)
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(stdin_text.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait rubash");
    finish(dir, output)
}

fn assert_run(run: &Run, expected_status: i32, expected_stdout: &str) {
    assert_eq!(
        run.status, expected_status,
        "status (stdout: {:?}, stderr: {})",
        run.stdout, run.stderr
    );
    assert_eq!(
        run.stdout, expected_stdout,
        "stdout (stderr: {})",
        run.stderr
    );
    let _ = std::fs::remove_dir_all(&run.dir);
}

/// The unit that sets the flag completes; no later unit is read.
#[test]
fn set_t_ends_script_after_its_own_unit() {
    let run = run_script("set -t\necho one\necho two\n", None, &[]);
    assert_run(&run, 0, "");
}

/// A mid-script `set -t`: earlier units ran, the flag's own unit
/// completes, everything after it is never read.
#[test]
fn mid_script_set_t_stops_after_its_unit() {
    let run = run_script("echo zero\nset -t\necho one\necho two\n", None, &[]);
    assert_run(&run, 0, "zero\n");
}

/// The unit after the flag never runs, so its failure cannot become
/// the script's status; the run ends with the flag unit's status.
#[test]
fn unit_after_set_t_never_runs() {
    let run = run_script("set -t\nfalse\necho unreached\n", None, &[]);
    assert_run(&run, 0, "");
}

/// A compound command is one read unit: it completes in full
/// (the flag set inside still counts), then the reader stops.
#[test]
fn compound_completes_before_the_stop() {
    let run = run_script(
        "if true; then\n  set -t\n  echo in\nfi\necho after\n",
        None,
        &[],
    );
    assert_run(&run, 0, "in\n");
}

/// Read granularity is the unit, not the simple command: a `;` list
/// sharing the flag's line runs to completion before the stop.
#[test]
fn same_line_list_runs_fully_before_the_stop() {
    let run = run_script("set -t; echo a\necho c\n", None, &[]);
    assert_run(&run, 0, "a\n");
}

/// The invocation door: `-t` presets the flag, so only the first
/// read unit of the script runs — both commands of its line, and
/// nothing of the next line.
#[test]
fn invocation_t_stops_after_first_unit() {
    let run = run_script("echo a; echo b\necho c\n", None, &["-t"]);
    assert_run(&run, 0, "a\nb\n");
}

/// With the flag preset, the first unit's failure IS the run's
/// status — the units after it are never read.
#[test]
fn invocation_t_status_is_first_units() {
    let run = run_script("false\necho x\n", None, &["-t"]);
    assert_run(&run, 1, "");
}

#[test]
fn invocation_t_second_unit_never_runs() {
    let run = run_script("echo a\nfalse\necho x\n", None, &["-t"]);
    assert_run(&run, 0, "a\n");
}

/// The non-interactive stdin reader is a principal reader too:
/// with `-t` preset, only its first unit runs.
#[test]
fn stdin_reader_honors_preset_onecmd() {
    let run = run_stdin(&["-t"], "echo s1\necho s2\n");
    assert_run(&run, 0, "s1\n");
}

/// A `set -t` typed into the stdin stream stops the stream reader
/// after its own unit.
#[test]
fn stdin_reader_honors_in_stream_set_t() {
    let run = run_stdin(&[], "echo s0\nset -t\necho s2\n");
    assert_run(&run, 0, "s0\n");
}

/// Guard: a `-c` string is parse_and_execute, not reader_loop —
/// `set -t` inside it stops nothing, neither inside the string nor
/// after it.
#[test]
fn command_string_ignores_onecmd() {
    let run = run_command_string("set -t; echo one; echo two");
    assert_run(&run, 0, "one\ntwo\n");
}

/// Guard: a sourced file's own reader takes no check — the child
/// runs to completion even though it sets the flag. The flag is
/// live afterwards, so the principal reader stops after the
/// sourcing unit: `m1`/`m2` are never read.
#[test]
fn sourced_file_runs_fully_but_stops_the_caller() {
    let run = run_script(
        ". ./child.sh\necho m1\necho m2\n",
        Some("set -t\necho c1\necho c2\n"),
        &[],
    );
    assert_run(&run, 0, "c1\nc2\n");
}

/// Guard: an eval string takes no check inside itself, but the
/// flag it sets stops the principal reader after the eval unit.
#[test]
fn eval_string_runs_fully_but_stops_the_caller() {
    let run = run_script("eval 'set -t; echo e1; echo e2'\necho m1\n", None, &[]);
    assert_run(&run, 0, "e1\ne2\n");
}

/// Guard, multi-line form: the eval string's own units are
/// parse_and_execute units, not reader units — every one of them
/// runs even after the string's first line sets the flag.
#[test]
fn multiline_eval_string_runs_fully_but_stops_the_caller() {
    let run = run_script("eval 'set -t\necho e1\necho e2'\necho m1\n", None, &[]);
    assert_run(&run, 0, "e1\ne2\n");
}
