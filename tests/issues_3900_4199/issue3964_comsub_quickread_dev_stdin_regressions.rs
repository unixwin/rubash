//! rubash#3964 (face 1): the `$(< file)` quick-read — GNU subst.c
//! optimize_cat_file over open_redir_file — resolved an fd-0 alias
//! target (`/dev/stdin`, `/dev/fd/0`) only through the fd table
//! (buffered snapshot / live File endpoint). When the enclosing
//! context's stdin rides the FUNCTION_STDIN channel — a pipeline
//! stage's input for in-process stages (brace groups, subshells),
//! a function's stdin — the fd table carries no fd-0 endpoint, so
//! the ladder fell through to a filesystem read of the literal
//! `/dev/stdin`: the rubash PROCESS's own stdin (empty under
//! /dev/null; a hang while a live ambient stdin stays open), never
//! the context's stream. `read x < /dev/stdin` and
//! `$(cat /dev/stdin)` in the same positions already read the
//! channel; only the quick-read ladder was blind to it.
//!
//! GNU model: command_substitute forks (subst.c:7143), so the
//! substitution reads the fd 0 in effect in the ENCLOSING context,
//! sharing its offset; the enclosing simple command's OWN
//! redirections are not yet applied during its word expansion
//! (the `x=$(</dev/stdin) <<<hi` control below is empty in GNU
//! too — pinned so the fix cannot overreach into the command's
//! own redirect walk).
//!
//! Oracle: GNU bash 5.3.20, script files, byte-compared
//! (stdout/stderr/status).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3964a-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    std::fs::write(dir.join("data.txt"), "filedata\n").expect("write data.txt");
    dir
}

fn rubash_c(script: &str, dir: &Path) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn assert_run(script: &str, expected_stdout: &str) {
    let dir = tempdir("case");
    let run = rubash_c(script, &dir);
    assert_eq!(run.stdout, expected_stdout, "stdout for {script:?}");
    assert_eq!(run.stderr, "", "stderr for {script:?}");
    assert_eq!(run.code, Some(0), "status for {script:?}");
}

/// The pipeline-group position from the report: the stage input is
/// the context fd 0 the substitution must read.
#[test]
fn quickread_dev_stdin_in_pipeline_group() {
    assert_run("echo hi | { x=$(</dev/stdin); echo \"[$x]\"; }", "[hi]\n");
}

/// A subshell pipeline stage feeds its body through the same
/// channel.
#[test]
fn quickread_dev_stdin_in_pipeline_subshell() {
    assert_run("echo hi | (x=$(</dev/stdin); echo \"[$x]\")", "[hi]\n");
}

/// The function position from the report: the call's here-string
/// binds the function's stdin.
#[test]
fn quickread_dev_stdin_in_function_herestring() {
    assert_run("f(){ x=$(</dev/stdin); echo \"[$x]\"; }; f <<<hi", "[hi]\n");
}

/// A function as a pipeline stage reads the stage input.
#[test]
fn quickread_dev_stdin_in_function_pipeline_stage() {
    assert_run(
        "f(){ x=$(</dev/stdin); echo \"[$x]\"; }; echo hi | f",
        "[hi]\n",
    );
}

/// The `/dev/fd/0` spelling names the same fd 0.
#[test]
fn quickread_dev_fd0_in_pipeline_group() {
    assert_run("echo hi | { x=$(</dev/fd/0); echo \"[$x]\"; }", "[hi]\n");
}

/// Shared cursor: a `read` inside the function consumes the first
/// line; the substitution sees only the remainder (GNU's shared
/// fd 0 offset).
#[test]
fn quickread_dev_stdin_after_read_shares_cursor() {
    assert_run(
        "f(){ read a; x=$(</dev/stdin); echo \"[$a|$x]\"; }; printf 'l1\\nl2\\n' | f",
        "[l1|l2]\n",
    );
}

/// The substitution drains the context stream: a later `read`
/// in the same function sees EOF.
#[test]
fn quickread_dev_stdin_drains_for_later_read() {
    assert_run(
        "f(){ x=$(</dev/stdin); read a; echo \"[$x|$a]\"; }; printf 'l1\\nl2\\n' | f",
        "[l1\nl2|]\n",
    );
}

/// Control: an enclosing redirect that rebinds fd 0 to a real
/// file shadows the channel — the file is read, not the
/// function's stdin.
#[test]
fn quickread_dev_stdin_group_redirect_file_shadows_channel() {
    assert_run(
        "f(){ { x=$(</dev/stdin); echo \"[$x]\"; } < data.txt; }; f <<<hi",
        "[filedata]\n",
    );
}

/// Control: the plain-file quick-read is unchanged.
#[test]
fn quickread_plain_file_unchanged() {
    assert_run("x=$(<data.txt); echo \"[$x]\"", "[filedata]\n");
}

/// Control (GNU order): the assignment command's OWN here-string
/// is not yet applied while its value expands — GNU yields empty
/// here too.
#[test]
fn quickread_own_herestring_not_yet_applied() {
    assert_run("x=$(</dev/stdin) <<<hi; echo \"[$x]\"", "[]\n");
}

/// The substitution must never touch the shell PROCESS's stdin:
/// with the process stdin held open (a pipe nobody writes to),
/// the pipeline-group form still completes from the stage input.
/// On the defective ladder this read the process's /dev/stdin
/// and blocked until EOF.
#[test]
fn quickread_dev_stdin_never_reads_process_stdin() {
    let dir = tempdir("held");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", "echo hi | { x=$(</dev/stdin); echo \"[$x]\"; }"])
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    // Hold the process stdin open without writing: take the writer
    // into a thread that simply keeps it alive past the deadline.
    let held_stdin = child.stdin.take().expect("child stdin");
    let holder = std::thread::spawn(move || {
        let mut held_stdin = held_stdin;
        std::thread::sleep(Duration::from_secs(30));
        let _ = held_stdin.write_all(b"");
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break Some(status),
            None => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    };
    drop(holder);
    let status = status.expect("rubash blocked reading its own process stdin");
    assert!(status.success(), "status {status:?}");
    let output = child.wait_with_output().expect("collect output");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "[hi]\n",
        "stdout with held-open process stdin"
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "",
        "stderr with held-open process stdin"
    );
}
