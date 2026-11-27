//! Issue rubash#426 (wt92/harvest divergence group 6): a sourced file whose
//! body runs `exit N` used to leave rubash's interactive session alive —
//! the sourced-exit path surfaced as a script-level status instead of a
//! session-level exit jump, so `niu -i` kept prompting and read the rest of
//! stdin. GNU bash terminates the whole shell: builtins/source.def sources
//! through evalstring.c parse_and_execute in the CURRENT shell, so the
//! `exit` builtin's jump_to_top_level(EXITPROG) unwinds past evalfile into
//! reader_loop; the shell ends with N (interactive REPL exit) or the -c
//! process exits N.
//!
//! GNU anchors (bash.git @b4608166, Bash-5.3 patch 15):
//! - builtins/source.def source_builtin -> evalfile.c evalfile_internal
//!   (FEVAL_BUILTIN) -> evalstring.c parse_and_execute: no SEVAL sep on the
//!   exit jump — EXITPROG propagates (evalstring.c:396-403 catch, :618-619
//!   re-raise) out of the sourced file into the caller's top level.
//! - builtins/exit.def exit_builtin -> exit_shell(JUMP_STATUS): the jump
//!   ends reader_loop (eval.c reader_loop) with last_command_exit_value = N;
//!   the interactive exit.def:59-62 "exit" echo fires once (reader phase).
//! - `return` stays a source-local unwind (builtins/return.def; unwind
//!   eval.c: parse.y simple_command -> return_builtin), and evalfile.c:395
//!   still runs the RETURN trap on the way out — only `exit` kills the
//!   shell.
//! - trap EXIT: exit_shell runs run_exit_trap before the final status
//!   (eval.c exit_shell -> run_exit_traps), so a sourcer-level EXIT trap
//!   fires with the sourced exit's status and cannot resurrect the shell.
//! - nested `.`: the inner file's exit unwinds both source frames (GNU
//!   `.` frames are unwind-protects, not exit barriers).
//!
//! The engine side rides the rubash#433 grouped-runner contract: the
//! sourced group's ExecuteError::ExitCode marks exit_jump_pending
//! (script_driver.rs run_source_impl) and the interactive reader ends the
//! session on the jump (run_interactive_stdin accept_line). These tests pin
//! the GNU-visible shapes so the sourced-exit path cannot regress into a
//! script-exit again.
#![cfg(windows)]

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Writes the sourced fixtures under a unique temp dir and returns it. Each
/// test owns its directory (process id + test tag), so parallel cargo test
/// runs never collide.
fn fixture_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-it426-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    dir
}

fn write_fixture(dir: &PathBuf, name: &str, body: &str) {
    std::fs::write(dir.join(name), body).expect("write fixture");
}

fn rubash_command(dir: &PathBuf) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.current_dir(dir).env_remove("PS1").env_remove("PS2");
    cmd
}

/// `-c` mode: the sourced `exit N` terminates the process with N and the
/// text after the `source` never runs (shell.c run_one_command -> exit N).
#[test]
fn c_mode_sourced_exit_n_terminates_with_n() {
    let dir = fixture_dir("c3");
    write_fixture(&dir, "exit3.sh", "exit 3\n");
    let output = rubash_command(&dir)
        .args(["-c", "echo BEFORE; source ./exit3.sh; echo AFTER"])
        .output()
        .expect("run rubash -c");
    assert_eq!(output.status.code(), Some(3));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "BEFORE\n");
}

/// The `exit 0` shape from the issue: the shell still terminates (rc 0) and
/// the trailing text never runs — the jump is not swallowed by the zero.
#[test]
fn c_mode_sourced_exit_zero_stops_execution() {
    let dir = fixture_dir("c0");
    write_fixture(&dir, "exit0.sh", "exit 0\n");
    let output = rubash_command(&dir)
        .args(["-c", "echo BEFORE; source ./exit0.sh; echo AFTER"])
        .output()
        .expect("run rubash -c");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "BEFORE\n");
}

/// Nested `.`: the inner file's exit unwinds both source frames; the outer
/// file's tail and the sourcer's continuation never run.
#[test]
fn nested_source_exit_terminates_the_shell() {
    let dir = fixture_dir("nested");
    write_fixture(&dir, "inner.sh", "exit 7\n");
    write_fixture(&dir, "outer.sh", "source ./inner.sh\necho OUTER-TAIL\n");
    let output = rubash_command(&dir)
        .args(["-c", "echo BEFORE; source ./outer.sh; echo AFTER"])
        .output()
        .expect("run rubash -c");
    assert_eq!(output.status.code(), Some(7));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "BEFORE\n");
}

/// `exit N` inside a function called from a sourced file is still a
/// shell exit (execute_cmd.c execute_shell_function does not barrier
/// EXITPROG).
#[test]
fn exit_inside_sourced_function_terminates_the_shell() {
    let dir = fixture_dir("fn");
    write_fixture(
        &dir,
        "fn.sh",
        "killshell() { exit 9; }\nkillshell\necho FN-TAIL\n",
    );
    let output = rubash_command(&dir)
        .args(["-c", "echo BEFORE; source ./fn.sh; echo AFTER"])
        .output()
        .expect("run rubash -c");
    assert_eq!(output.status.code(), Some(9));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "BEFORE\n");
}

/// `return N` keeps its source-local semantics: sourcing stops, the shell
/// lives on, and `.`'s status is N (visible as $? on the next command).
#[test]
fn return_in_sourced_file_only_stops_the_source() {
    let dir = fixture_dir("ret");
    write_fixture(&dir, "ret.sh", "echo IN-SRC\nreturn 4\necho SRC-TAIL\n");
    let output = rubash_command(&dir)
        .args(["-c", "source ./ret.sh; echo AFTER-RC=$?"])
        .output()
        .expect("run rubash -c");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("IN-SRC"), "{stdout:?}");
    assert!(!stdout.contains("SRC-TAIL"), "{stdout:?}");
    assert!(stdout.contains("AFTER-RC=4"), "{stdout:?}");
}

/// eval.c exit_shell -> run_exit_traps: the EXIT trap fires before the
/// shell dies, sees the sourced exit's status, and cannot keep the shell
/// alive.
#[test]
fn exit_trap_runs_before_a_sourced_exit_terminates() {
    let dir = fixture_dir("trap");
    write_fixture(&dir, "exit5.sh", "exit 5\n");
    let output = rubash_command(&dir)
        .args([
            "-c",
            "trap 'echo BYE-RC=$?' EXIT; source ./exit5.sh; echo AFTER",
        ])
        .output()
        .expect("run rubash -c");
    assert_eq!(output.status.code(), Some(5));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "BYE-RC=5\n");
}

struct Session {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// Piped-`-i` interactive session: `rubash -i --norc` with the given lines
/// on stdin, cwd at the fixture dir (the rubash#433 probe shape).
fn run_interactive(dir: &PathBuf, lines: &str) -> Session {
    let mut child = rubash_command(dir)
        .arg("-i")
        .arg("--norc")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash -i");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(lines.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    Session {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// The issue's headline shape: `source` a file with `exit 0` at the prompt —
/// the session ends before the next line is read or run (one prompt total,
/// rc 0). GNU piped-`-i`: `. f` with `exit 0` inside kills the shell.
#[test]
fn interactive_sourced_exit_zero_ends_the_session() {
    let dir = fixture_dir("i0");
    write_fixture(&dir, "exit0.sh", "exit 0\n");
    let session = run_interactive(&dir, "source ./exit0.sh\necho STILL-ALIVE\n");
    assert_eq!(session.code, Some(0), "stderr: {}", session.stderr);
    assert!(
        !session.stdout.contains("STILL-ALIVE"),
        "the session must not survive the sourced exit: {:?} {:?}",
        session.stdout,
        session.stderr
    );
    assert_eq!(
        session.stderr.matches("bash-5.3#").count(),
        1,
        "exactly one prompt is rendered before the shell dies: {:?}",
        session.stderr
    );
}

/// The jump carries N: `source` of an `exit 5` file ends the REPL with rc 5
/// and the trailing input is never read.
#[test]
fn interactive_sourced_exit_n_uses_the_given_status() {
    let dir = fixture_dir("i5");
    write_fixture(&dir, "exit5.sh", "exit 5\n");
    let session = run_interactive(&dir, "echo BEFORE\nsource ./exit5.sh\necho STILL-ALIVE\n");
    assert_eq!(session.code, Some(5), "stderr: {}", session.stderr);
    assert_eq!(session.stdout, "BEFORE\n", "stderr: {}", session.stderr);
    assert!(
        !session.stderr.contains("STILL-ALIVE"),
        "the line after the sourced exit must never be echoed or run: {:?}",
        session.stderr
    );
}

/// `return` at an interactive prompt's sourced file only stops the source:
/// the session survives, prompts again, and runs the next command with the
/// return status in $?.
#[test]
fn interactive_return_in_sourced_file_keeps_the_session_alive() {
    let dir = fixture_dir("iret");
    write_fixture(&dir, "ret.sh", "return 4\necho SRC-TAIL\n");
    let session = run_interactive(&dir, "source ./ret.sh\necho ALIVE-RC=$?\n");
    assert_eq!(session.code, Some(0), "stderr: {}", session.stderr);
    assert!(
        session.stdout.contains("ALIVE-RC=4"),
        "the session must survive `return` inside a sourced file: {:?} {:?}",
        session.stdout,
        session.stderr
    );
    assert!(
        !session.stdout.contains("SRC-TAIL"),
        "return must still stop the remaining sourced text: {:?}",
        session.stdout
    );
}

/// The EXIT trap fires at the sourced exit's boundary inside the dying
/// interactive session, reports the sourced status, and the shell still
/// ends with that status.
#[test]
fn interactive_exit_trap_runs_before_a_sourced_exit() {
    let dir = fixture_dir("itrap");
    write_fixture(&dir, "exit9.sh", "exit 9\n");
    let session = run_interactive(
        &dir,
        "trap 'echo BYE-RC=$?' EXIT\nsource ./exit9.sh\necho STILL-ALIVE\n",
    );
    assert_eq!(session.code, Some(9), "stderr: {}", session.stderr);
    assert!(
        session.stdout.contains("BYE-RC=9"),
        "the EXIT trap must fire with the sourced exit's status: {:?} {:?}",
        session.stdout,
        session.stderr
    );
    assert!(
        !session.stdout.contains("STILL-ALIVE"),
        "the trap must not resurrect the session: {:?}",
        session.stdout
    );
}

/// Nested `.` at the prompt: the inner file's exit ends the session; the
/// outer file's tail never runs.
#[test]
fn interactive_nested_source_exit_ends_the_session() {
    let dir = fixture_dir("inested");
    write_fixture(&dir, "inner.sh", "exit 7\n");
    write_fixture(&dir, "outer.sh", "source ./inner.sh\necho OUTER-TAIL\n");
    let session = run_interactive(&dir, "source ./outer.sh\necho STILL-ALIVE\n");
    assert_eq!(session.code, Some(7), "stderr: {}", session.stderr);
    assert!(
        !session.stdout.contains("OUTER-TAIL"),
        "the outer sourced tail must never run: {:?} {:?}",
        session.stdout,
        session.stderr
    );
}
