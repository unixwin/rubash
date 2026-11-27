//! Source-boundary parse-abort regressions (rubash#451 class B).
//!
//! A sourced file whose tail carries an incomplete construct (unclosed
//! `if`/function body, trailing `|`/`&&`, unclosed backtick block) ends in a
//! parse error. GNU's evalstring.c parse_and_execute aborts the remaining
//! sourced text as an ORDINARY failure (evalstring.c:585-601 — no
//! jump_to_top_level), so the sourcing driver keeps reading: an interactive
//! shell reports the diagnostic and PROMPTS AGAIN (verified WSL GNU 5.3.0),
//! and a non-interactive script continues with its next command.
//!
//! The engine recorded the parse abort's ExitCode(2) as a reader exit jump
//! (run_source_impl's execute_ast arm) while the `source` builtin consumed
//! the parse-error flag that classifies it, so every driver gate of the
//! shape `exit_jump && !parse_error` read a reader-discard as a real `exit`:
//! the interactive session died right after the diagnostic (the eco
//! `hang:enter1` family — the first probe line after `source` never ran),
//! and non-interactive scripts aborted with rc 2 where GNU continues.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

/// Unique per-test temp file with forward slashes (the interactive arg
/// parser treats `\` as an escape, GNU paths accept `/`).
fn temp_script(name: &str, body: &str) -> String {
    let path = std::env::temp_dir().join(format!("rubash451_{}_{}.sh", name, std::process::id()));
    std::fs::write(&path, body).expect("write temp script");
    path.to_string_lossy().replace('\\', "/")
}

fn drop_script(path: &str) {
    let _ = std::fs::remove_file(path);
}

fn run_stdin_with_timeout(stdin_text: &str) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(["-i"])
        .env_remove("PS1")
        .env_remove("PS2")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(stdin_text.as_bytes())
        .expect("write stdin");
    drop(child.stdin.take());
    // The family this pins used to kill the reader right after the
    // diagnostic; a missing exit must fail the test, not the suite runner.
    for _ in 0..300 {
        match child.try_wait().expect("wait rubash") {
            Some(_) => break,
            None => std::thread::sleep(Duration::from_millis(100)),
        }
    }
    let output = child.wait_with_output().expect("wait rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// After a sourced file aborts on an incomplete tail, the interactive
/// session must still run the next line (GNU: report, discard, prompt).
#[test]
fn interactive_session_survives_sourced_unclosed_if_tail() {
    let path = temp_script("if", "echo START\nif true; then\n  echo t\n");
    let (stdout, stderr, code) =
        run_stdin_with_timeout(&format!("source {path}\necho ECOSRC\nexit\n"));
    drop_script(&path);
    assert!(stdout.contains("START"), "prefix group lost: {stdout:?}");
    assert!(
        stdout.contains("ECOSRC"),
        "session died after source: {stdout:?}"
    );
    assert!(
        stderr.contains("unexpected end of file"),
        "diagnostic missing: {stderr:?}"
    );
    assert_eq!(code, Some(0), "{code:?}");
}

#[test]
fn interactive_session_survives_sourced_unclosed_function_tail() {
    let path = temp_script("func", "echo START\nf() {\n");
    let (stdout, _stderr, code) =
        run_stdin_with_timeout(&format!("source {path}\necho ECOSRC\nexit\n"));
    drop_script(&path);
    assert!(stdout.contains("START"), "prefix group lost: {stdout:?}");
    assert!(
        stdout.contains("ECOSRC"),
        "session died after source: {stdout:?}"
    );
    assert_eq!(code, Some(0), "{code:?}");
}

#[test]
fn interactive_session_survives_sourced_trailing_pipe_tail() {
    let path = temp_script("pipe", "echo START\necho a |\n");
    let (stdout, _stderr, code) =
        run_stdin_with_timeout(&format!("source {path}\necho ECOSRC\nexit\n"));
    drop_script(&path);
    assert!(stdout.contains("START"), "prefix group lost: {stdout:?}");
    assert!(
        stdout.contains("ECOSRC"),
        "session died after source: {stdout:?}"
    );
    assert_eq!(code, Some(0), "{code:?}");
}

#[test]
fn interactive_session_survives_sourced_trailing_andand_tail() {
    let path = temp_script("andand", "echo START\necho a &&\n");
    let (stdout, _stderr, code) =
        run_stdin_with_timeout(&format!("source {path}\necho ECOSRC\nexit\n"));
    drop_script(&path);
    assert!(stdout.contains("START"), "prefix group lost: {stdout:?}");
    assert!(
        stdout.contains("ECOSRC"),
        "session died after source: {stdout:?}"
    );
    assert_eq!(code, Some(0), "{code:?}");
}

/// doitlive walkthrough.sh shape: an unclosed ``` fence opens a backquote
/// command substitution that runs to EOF; GNU reports the comsub syntax
/// error and the next interactive command still runs (the comsub's child
/// commands aside, the READER survives). The body word is deliberately a
/// command no machine has, so the probe line cannot be eaten by a real
/// REPL child spawning inside the substitution.
#[test]
fn interactive_session_survives_sourced_unclosed_backtick_block() {
    let path = temp_script(
        "backtick",
        "echo 'before'\n```no-such-python-451\nlist = [2, 4, 6, 8]\n",
    );
    let (stdout, stderr, code) =
        run_stdin_with_timeout(&format!("source {path}\necho ECOSRC\nexit\n"));
    drop_script(&path);
    assert!(stdout.contains("before"), "prefix group lost: {stdout:?}");
    assert!(
        stdout.contains("ECOSRC"),
        "session died after source: {stdout:?}"
    );
    assert!(
        stderr.contains("``") || stderr.contains("unexpected"),
        "diagnostic missing: {stderr:?}"
    );
    assert_eq!(code, Some(0), "{code:?}");
}

/// The piped driver keeps reading after a sourced file's syntax error
/// (`printf 'source bad\necho AFTER\n' | bash` prints AFTER, rc 0) — the
/// sourcing driver must not abort with rc 2.
#[test]
fn piped_driver_continues_after_sourced_parse_error() {
    let path = temp_script("nonint", "if true; then\n");
    let (stdout, _stderr, code) = run_stdin_with_timeout(&format!("source {path}\necho AFTER\n"));
    drop_script(&path);
    assert!(stdout.contains("AFTER"), "script aborted: {stdout:?}");
    assert_eq!(code, Some(0), "{code:?}");
}

/// A REAL `exit` inside the sourced file is still an exit-shell jump: the
/// session ends with the exit status and the caller's next line never runs.
#[test]
fn real_exit_inside_sourced_file_still_ends_the_session() {
    let path = temp_script("exit", "echo BEFORE\nexit 5\n");
    let (stdout, _stderr, code) =
        run_stdin_with_timeout(&format!("source {path}\necho ECOSRC\nexit\n"));
    drop_script(&path);
    assert!(stdout.contains("BEFORE"), "prefix group lost: {stdout:?}");
    assert!(
        !stdout.contains("ECOSRC"),
        "exit was downgraded to a discard: {stdout:?}"
    );
    assert_eq!(code, Some(5), "{code:?}");
}

/// An `exit` in an early group followed by a parse error in a later group
/// of the same file keeps the exit-shell semantics (the unwind wins).
#[test]
fn exit_before_later_parse_error_still_ends_the_session() {
    let path = temp_script("exitthenbad", "echo BEFORE\nexit 7\nif true; then\n");
    let (stdout, _stderr, code) =
        run_stdin_with_timeout(&format!("source {path}\necho ECOSRC\nexit\n"));
    drop_script(&path);
    assert!(stdout.contains("BEFORE"), "prefix group lost: {stdout:?}");
    assert!(
        !stdout.contains("ECOSRC"),
        "exit was downgraded to a discard: {stdout:?}"
    );
    assert_eq!(code, Some(7), "{code:?}");
}
