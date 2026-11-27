//! rubash#1822: the shell's virtual-stdin feed raced a fast-exiting
//! external child. The executor materializes a compound stage's input
//! as FUNCTION_STDIN text and, after spawning an external command
//! inside the stage, writes that payload into the child's stdin pipe.
//! A child that never reads stdin and exits quickly (readlink) closed
//! the pipe's read end first; the feed write then failed with EPIPE
//! and the error was propagated as the COMMAND's failure — status 1,
//! a spurious `Broken pipe` diagnostic, and the child's real output
//! and exit status discarded (`|| echo nolink` fired on a command
//! that had in fact succeeded). Outside a pipeline the same feed ran
//! with SIGPIPE at its process-wide SIG_DFL disposition, so the race
//! killed the shell itself (exit 141) on `cmd <<< word`.
//!
//! GNU has no post-spawn feed to race: the element's fd 0 IS the pipe
//! (execute_cmd.c:2620 execute_pipeline), a consumer that exits
//! without reading leaves the bytes in the pipe, and its own exit
//! status governs. Oracle: GNU bash 5.3.20 — seed shape 0/40 bad,
//! here-string shape 0/40 bad under the same env -i conditions.
//!
//! The pins loop: each iteration is an independent race trial, and on
//! the fixed tree every iteration is deterministic — the feed can no
//! longer fail the command or the shell.

#![cfg(unix)]

use std::process::{Command, Stdio};

/// Local stand-in for the `tempfile` crate (not a dev-dependency of
/// this repo): a unique scratch directory removed on drop.
struct ScratchDir(std::path::PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch_tempdir() -> ScratchDir {
    let dir = std::env::temp_dir().join(format!(
        "rubash-scratch-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    ScratchDir(dir)
}

fn rubash_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_rubash"))
}

/// Write `case.sh` holding `script` into a scratch dir, run it under
/// rubash with a cleared environment (the issue's reproduction
/// conditions), and return (stdout, stderr, success).
fn run_script(script: &str) -> (String, String, bool) {
    let dir = scratch_tempdir();
    let script_path = dir.0.join("case.sh");
    std::fs::write(&script_path, script).expect("write script");
    let output = Command::new(rubash_bin())
        .arg(&script_path)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", &dir.0)
        .current_dir(&dir.0)
        .stdin(Stdio::null())
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.success(),
    )
}

#[test]
fn issue1822_pipeline_group_child_output_and_status_survive() {
    // The issue seed verbatim: readlink succeeds and prints the pipe
    // target; the `||` fallback must never fire.
    for _ in 0..30 {
        let (stdout, stderr, success) =
            run_script("echo hello | { readlink /proc/self/fd/1 || echo nolink; } | cat\n");
        assert!(success, "shell must exit 0, stderr: {stderr}");
        assert!(
            stdout.starts_with("pipe:["),
            "child output must reach the pipeline, got: {stdout}"
        );
        assert!(
            !stdout.contains("nolink"),
            "fallback fired for a successful child: {stdout}"
        );
        assert!(stderr.is_empty(), "no diagnostic expected: {stderr}");
    }
}

#[test]
fn issue1822_pipeline_group_child_status_is_zero() {
    // The status the group observes for the child is the child's own.
    for _ in 0..20 {
        let (stdout, stderr, success) =
            run_script("echo hello | { readlink /proc/self/fd/1 >/dev/null; echo st=$?; } | cat\n");
        assert!(success, "shell must exit 0, stderr: {stderr}");
        assert_eq!(stdout, "st=0\n", "child status must be 0: {stdout}");
        assert!(stderr.is_empty(), "no diagnostic expected: {stderr}");
    }
}

#[test]
fn issue1822_herestring_fast_exit_child_does_not_kill_shell() {
    // Non-pipeline face of the same feed: a here-string payload fed to
    // a child that exits without reading must not SIGPIPE the shell.
    for _ in 0..30 {
        let (stdout, stderr, success) =
            run_script("readlink /proc/self/fd/1 <<< hello\necho rc=$?\n");
        assert!(success, "shell must survive and exit 0, stderr: {stderr}");
        assert!(
            stdout.contains("rc=0"),
            "child status 0 and the script continues: {stdout}"
        );
        assert!(stderr.is_empty(), "no diagnostic expected: {stderr}");
    }
}
