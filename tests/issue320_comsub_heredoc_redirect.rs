//! Issue rubash#320 regression: a heredoc-carrying command inside a command
//! substitution must not lose file redirections that share the command.
//!
//! GNU binds redirections before the command runs (redir.c
//! do_redirection_internal), so `cat <<EOF > /dev/null` inside `$( )` reads
//! the heredoc on stdin and writes the capture to /dev/null — the
//! substitution captures nothing. Rubash's `cat <<EOF` comsub shortcut
//! modeled only the heredoc and dropped every other redirect, so
//! `$(cat <<EOF > /dev/null)` captured the body text (probe b2 of the
//! fix16 lane). The shortcut now declines commands carrying file
//! redirections (operator not starting with `<<`) and the real executor
//! applies them.

use std::io::Write;
use std::process::Command;

fn run_script(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i320-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&path).expect("create probe");
    file.write_all(script.as_bytes()).expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .env("TMPDIR", &dir)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Redirect after the heredoc operator: capture is empty (probe b2a).
#[test]
fn heredoc_then_dev_null_redirect_captures_nothing() {
    let (stdout, stderr, rc) =
        run_script("x=$(cat <<EOF > /dev/null\nhello\nEOF\n)\necho \"x=[$x]\"\n");
    assert_eq!(stdout, "x=[]\n", "stderr: {stderr}");
    assert_eq!(rc, Some(0));
}

/// Redirect before the heredoc operator: same result (probe b2c).
#[test]
fn dev_null_redirect_then_heredoc_captures_nothing() {
    let (stdout, stderr, rc) =
        run_script("x=$(cat > /dev/null <<EOF\nhello\nEOF\n)\necho \"x=[$x]\"\n");
    assert_eq!(stdout, "x=[]\n", "stderr: {stderr}");
    assert_eq!(rc, Some(0));
}

/// The redirect target receives the body (probe b2e).
#[test]
fn heredoc_redirect_to_file_writes_the_file() {
    let (stdout, stderr, rc) = run_script(
        "x=$(cat <<EOF > out320.txt\nhello\nEOF\n)\necho \"x=[$x]\"\ncat out320.txt\nrm -f out320.txt\n",
    );
    assert_eq!(stdout, "x=[]\nhello\n", "stderr: {stderr}");
    assert_eq!(rc, Some(0));
}

/// A stderr-only redirect does not touch the capture (probe b2d): the body
/// still lands in the substitution.
#[test]
fn heredoc_with_stderr_redirect_keeps_capture() {
    let (stdout, stderr, rc) =
        run_script("x=$(cat <<EOF 2> /dev/null\nhello\nEOF\n)\necho \"x=[$x]\"\n");
    assert_eq!(stdout, "x=[hello]\n", "stderr: {stderr}");
    assert_eq!(rc, Some(0));
}

/// Pipeline with a redirect on the piped stage (probe b2g).
#[test]
fn heredoc_pipe_sort_redirect_captures_nothing() {
    let (stdout, stderr, rc) =
        run_script("x=$(cat <<EOF | sort -u > /dev/null\nb\na\nb\nEOF\n)\necho \"x=[$x]\"\n");
    assert_eq!(stdout, "x=[]\n", "stderr: {stderr}");
    assert_eq!(rc, Some(0));
}

/// Argument-position comsub, redirect dropped shape (probe b2f).
#[test]
fn argument_position_heredoc_redirect_captures_nothing() {
    let (stdout, stderr, rc) = run_script("echo \"[$(cat <<EOF > /dev/null\nhi\nEOF\n)]\"\n");
    assert_eq!(stdout, "[]\n", "stderr: {stderr}");
    assert_eq!(rc, Some(0));
}

/// The plain heredoc capture (no redirect) still works through the
/// shortcut (probe b2b).
#[test]
fn heredoc_without_redirect_still_captures() {
    let (stdout, stderr, rc) = run_script("x=$(cat <<EOF\nhello\nEOF\n)\necho \"x=[$x]\"\n");
    assert_eq!(stdout, "x=[hello]\n", "stderr: {stderr}");
    assert_eq!(rc, Some(0));
}
