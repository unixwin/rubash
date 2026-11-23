//! An escaped blank (`a\ b`) lexes to the ANSI_C_IFS_GUARD carrier plus
//! the blank. The in-process pipeline stage arms (`cat`, and the arms
//! sharing `expand_pipeline_stage_arg_words`) expand their own operands
//! and used to leave the guard in the argv, so `cat a\ b | wc -l` tried to
//! open `a<U+E401> b` and failed with "No such file or directory". Seen
//! in niubash 1.5.1 through the interactive reader (`-i`).

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

mod common;
use common::wait_bounded;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-escblank-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

/// Run `input` through the pipe-driven interactive form; returns
/// (stdout, stderr).
fn run_interactive(input: &str, tag: &str) -> (String, String) {
    let dir = tempdir(tag);
    let home = dir.join("home");
    std::fs::create_dir_all(&home).expect("create home");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-i")
        .current_dir(&dir)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("TERM", "xterm")
        .env("HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash -i");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("feed input");
    let outcome = wait_bounded(&mut child, Duration::from_secs(60));
    assert!(!outcome.timed_out, "child {} timed out", outcome.pid);
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&outcome.stdout).into_owned(),
        String::from_utf8_lossy(&outcome.stderr).into_owned(),
    )
}

#[test]
fn cat_stage_operand_with_escaped_blank_opens_the_file() {
    let (stdout, stderr) = run_interactive(
        concat!(
            "echo xxx > a\\ b\n",
            "cat a\\ b | wc -l\n",
            "cat a\\ b | cat\n",
            "cat a\\ b|cat\n",
            "exit\n",
        ),
        "cat",
    );
    assert!(!stderr.contains("No such file"), "stderr: {stderr:?}");
    assert!(stdout.contains("xxx"), "stdout: {stdout:?}");
    assert_eq!(stdout.matches("xxx").count(), 2, "stdout: {stdout:?}");
    assert!(
        stdout.lines().any(|l| l.trim() == "1"),
        "stdout: {stdout:?}"
    );
}

#[test]
fn stage_operands_with_escaped_blank_carry_no_guard() {
    // echo/printf stages take the shared operand helper: the argv must be
    // the plain `a b`, with no U+E401 left in front of the blank.
    let (stdout, _stderr) = run_interactive(
        concat!(
            "echo a\\ b | od -An -c\n",
            "printf '%s\\n' a\\ b | od -An -c\n",
            "exit\n",
        ),
        "echo",
    );
    assert!(!stdout.contains('\u{e401}'), "stdout: {stdout:?}");
    assert_eq!(
        stdout.matches("a       b  \\n").count(),
        2,
        "stdout: {stdout:?}"
    );
}

#[test]
fn cat_stage_operand_with_escaped_blank_under_debug_trap() {
    // niubash 1.5.1 reproducer: oh-my-niu installs a DEBUG trap for its
    // preexec hook, and with a DEBUG trap armed an interactive pipeline's
    // `cat` stage kept the guard in its operand. Without the trap the
    // stage took a path that stripped it, so only the rc'd shell failed.
    let (stdout, stderr) = run_interactive(
        concat!(
            "trap 'true' DEBUG\n",
            "echo xxx > a\\ b\n",
            "cat a\\ b | wc -l\n",
            "cat a\\ b | cat\n",
            "exit\n",
        ),
        "debugtrap",
    );
    assert!(!stderr.contains("No such file"), "stderr: {stderr:?}");
    assert_eq!(stdout.matches("xxx").count(), 1, "stdout: {stdout:?}");
    assert!(
        stdout.lines().any(|l| l.trim() == "1"),
        "stdout: {stdout:?}"
    );
}
