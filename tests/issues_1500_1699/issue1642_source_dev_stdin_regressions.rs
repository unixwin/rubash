//! Issue rubash#1642: `source /dev/stdin` read nothing when stdin was
//! bound by a file or here-string redirection — GNU executes the
//! redirected text. GNU source.def opens /dev/stdin, a dup of fd 0, so
//! the bytes are whatever fd 0 is bound to for that command: its own
//! `< file` / `<<<` / `<<EOF` redirections, an enclosing scope's fd-0
//! binding, or the pipeline stage buffer. The builtin only consulted
//! the pipeline buffer and the process's own stdin handle, neither of
//! which carries a command-level redirection (it lives in the command
//! node / virtual fd table), so the redirected forms silently sourced
//! nothing with status 0.
//!
//! Assertions follow the GNU bash 5.3.20 behavior measured with the
//! same scripts.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1642-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
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

/// Run rubash with `payload` as the process stdin.
fn rubash_c_with_stdin(script: &str, dir: &Path, payload: &str) -> Run {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(payload.as_bytes())
        .expect("write stdin payload");
    let output = child.wait_with_output().expect("wait for rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn seed_source_file(dir: &Path) {
    std::fs::write(dir.join("s.sh"), "echo FROMFILE2\n").expect("seed s.sh");
}

#[test]
fn issue1642_source_dev_stdin_here_string() {
    let dir = tempdir("herestring");
    let run = rubash_c("source /dev/stdin <<< 'echo SRCHS'", &dir);
    assert_eq!(run.stdout, "SRCHS\n", "stderr: {}", run.stderr);
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue1642_source_dev_stdin_file_redirect() {
    let dir = tempdir("file");
    seed_source_file(&dir);
    let run = rubash_c("source /dev/stdin < s.sh", &dir);
    assert_eq!(run.stdout, "FROMFILE2\n", "stderr: {}", run.stderr);
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue1642_dot_dev_stdin_here_string() {
    let dir = tempdir("dot");
    let run = rubash_c(". /dev/stdin <<< 'echo DOTSRC'", &dir);
    assert_eq!(run.stdout, "DOTSRC\n", "stderr: {}", run.stderr);
}

#[test]
fn issue1642_source_dev_stdin_heredoc() {
    let dir = tempdir("heredoc");
    let run = rubash_c(
        "source /dev/stdin <<RUBASH_EOF\necho HDOC\nRUBASH_EOF",
        &dir,
    );
    assert_eq!(run.stdout, "HDOC\n", "stderr: {}", run.stderr);
}

#[test]
fn issue1642_source_dev_stdin_with_arguments() {
    let dir = tempdir("args");
    let run = rubash_c("source /dev/stdin a1 <<< 'echo \"arg=$1\"'", &dir);
    assert_eq!(run.stdout, "arg=a1\n", "stderr: {}", run.stderr);
}

#[test]
fn issue1642_source_dev_stdin_pipeline_control() {
    // Already-working control: the pipeline stage buffer carries stdin.
    let dir = tempdir("pipe");
    let run = rubash_c("echo 'echo SRCSTDIN' | source /dev/stdin", &dir);
    assert_eq!(run.stdout, "SRCSTDIN\n", "stderr: {}", run.stderr);
}

#[test]
fn issue1642_source_dev_stdin_process_stdin_control() {
    // Already-working control: with no redirection anywhere, /dev/stdin
    // is the process stdin itself.
    let dir = tempdir("procstdin");
    let run = rubash_c_with_stdin("source /dev/stdin", &dir, "echo VIA_PROCESS_STDIN\n");
    assert_eq!(run.stdout, "VIA_PROCESS_STDIN\n", "stderr: {}", run.stderr);
}

#[test]
fn issue1642_source_dev_stdin_function_scope_control() {
    // Already-working control: a function call's here-string binds fd 0
    // for the body.
    let dir = tempdir("function");
    let run = rubash_c("f() { source /dev/stdin; }; f <<< 'echo FNHERE'", &dir);
    assert_eq!(run.stdout, "FNHERE\n", "stderr: {}", run.stderr);
}

#[test]
fn issue1642_source_dev_stdin_compound_scope_control() {
    // Already-working control: a brace group's here-string binds fd 0
    // for the group.
    let dir = tempdir("compound");
    let run = rubash_c("{ source /dev/stdin; } <<< 'echo GRPHERE'", &dir);
    assert_eq!(run.stdout, "GRPHERE\n", "stderr: {}", run.stderr);
}

#[test]
fn issue1642_source_dev_stdin_missing_file_never_reads_process_stdin() {
    // The command's own fd-0 redirection claims stdin: when it cannot
    // be resolved (missing file), GNU never reaches a body — source
    // must not fall through to the process stdin and execute that.
    let dir = tempdir("missing");
    let run = rubash_c_with_stdin(
        "source /dev/stdin < missing.sh; echo after",
        &dir,
        "echo SHOULD_NOT_RUN\n",
    );
    assert!(
        !run.stdout.contains("SHOULD_NOT_RUN"),
        "stdout: {}",
        run.stdout
    );
}

#[test]
fn issue1642_source_dev_stdin_explicit_redirect_wins_over_pipe() {
    // Shape incorporated tests-only from the single-issue PR #1685 per
    // the 2026-10-26 fd-source ruling: the process stdin is a pipe, but
    // the command carries its own here-string — the explicit fd-0
    // redirection wins over the piped input (GNU: redir.c binds fd 0
    // before the builtin runs).
    let dir = tempdir("redirwins");
    let run = rubash_c_with_stdin(
        "source /dev/stdin <<< 'echo FROMREDIR'; echo \"rc=$?\"",
        &dir,
        "echo FROMPIPE\n",
    );
    assert_eq!(run.stdout, "FROMREDIR\nrc=0\n", "stderr: {}", run.stderr);
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue1642_source_dev_fd0_alias_here_string() {
    // Shape incorporated tests-only from the single-issue PR #1685 per
    // the 2026-10-26 fd-source ruling: /dev/fd/0 is an alias spelling of
    // the same fd-0 binding and must behave like /dev/stdin.
    let dir = tempdir("fd0alias");
    let run = rubash_c("source /dev/fd/0 <<< 'echo FD0'; echo \"rc=$?\"", &dir);
    assert_eq!(run.stdout, "FD0\nrc=0\n", "stderr: {}", run.stderr);
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue1642_source_plain_file_control() {
    let dir = tempdir("plain");
    seed_source_file(&dir);
    let run = rubash_c("source s.sh", &dir);
    assert_eq!(run.stdout, "FROMFILE2\n", "stderr: {}", run.stderr);
}
