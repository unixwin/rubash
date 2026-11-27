//! rubash#1877: after `exec < file`, an external command run inside a
//! command substitution did not share fd 0's open file description —
//! the single-external-command substitution fast path never consulted
//! the fd table, so the child inherited the shell PROCESS's stdin
//! (re-reading the script's first line, or EOF under /dev/null) and the
//! shell's own offset never advanced.
//!
//! GNU model: subst.c:7143 command_substitute forks, so the child
//! shares the parent's fd 0 — offset included — exactly like a plain
//! external command (apply_external_stdin_redirect's tail arms:
//! closed -> EBADF stdin, File/CoprocStdout -> duplicated handle,
//! buffered text -> remaining bytes + parent cursor drained).
//!
//! Oracle: GNU bash 5.3.20, script files, byte-compared. One shape is
//! NOT oracle-pinnable: `exec <&-; x=$(head -n 1)` hangs the oracle
//! itself (verified 2026-10-28, two variants, 6s+ timeouts) — the pin
//! for the closed-fd comsub arm therefore asserts the sane completion
//! (child fails EBADF, substitution empty, script continues) that the
//! fork model requires, and the top-level closed shape pins the
//! oracle's own observable behavior.

#![cfg(unix)]

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

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

/// Run `case.sh` (with `data3.txt` beside it) under rubash in its own
/// process group with a watchdog deadline; a hung run kills the whole
/// group so no orphan can hold the harness pipes. Returns
/// (stdout, stderr, success).
fn run_script_watchdog(script: &str, timeout: Duration) -> (String, String, bool) {
    use std::os::unix::process::CommandExt;
    let dir = scratch_tempdir();
    std::fs::write(dir.0.join("data3.txt"), "one\ntwo\nthree\n").expect("write data");
    let script_path = dir.0.join("case.sh");
    std::fs::write(&script_path, script).expect("write script");
    let mut child = Command::new(rubash_bin())
        .arg(&script_path)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", &dir.0)
        .current_dir(&dir.0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .expect("spawn rubash");
    let pgid = child.id();
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break Some(status),
            None => {
                if Instant::now() >= deadline {
                    let _ = Command::new("/bin/kill")
                        .arg("-KILL")
                        .arg(format!("-{pgid}"))
                        .status();
                    let _ = child.kill();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_end(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_end(&mut stderr);
    }
    let _ = child.wait();
    (
        String::from_utf8_lossy(&stdout).into_owned(),
        String::from_utf8_lossy(&stderr).into_owned(),
        status.is_some_and(|s| s.success()),
    )
}

#[test]
fn issue1877_comsub_external_shares_exec_stdin_offset() {
    // Issue seed shape: builtin read, two external substitutions, a
    // final builtin read — every reader advances the ONE offset.
    let (stdout, stderr, success) = run_script_watchdog(
        "exec < data3.txt\nread -r a\ny=$(head -n 1)\nz=$(head -n 1)\nread -r b\nprintf 'builtin: %s\\next1: %s\\next2: %s\\nbuiltin2: %s\\n' \"$a\" \"$y\" \"$z\" \"$b\"\n",
        Duration::from_secs(20),
    );
    assert!(success, "run must complete, stderr: {stderr}");
    assert_eq!(stdout, "builtin: one\next1: two\next2: three\nbuiltin2: \n");
    assert!(stderr.is_empty(), "no diagnostics expected: {stderr}");
}

#[test]
fn issue1877_comsub_external_first_reader_gets_file_head() {
    // External-first order: the first substitution takes line one; a
    // second substitution continues at line two — and a trailing
    // builtin read then sees EOF, proving the offset lives in the one
    // shared open file description, not in per-command copies.
    let (stdout, stderr, success) = run_script_watchdog(
        "exec < data3.txt\nx=$(head -n 1)\ny=$(head -n 2)\nread -r b\nprintf 'ext-first: %s\\next-rest: [%s]\\nbuiltin-tail: [%s]\\n' \"$x\" \"$y\" \"$b\"\n",
        Duration::from_secs(20),
    );
    assert!(success, "run must complete, stderr: {stderr}");
    assert_eq!(
        stdout,
        "ext-first: one\next-rest: [two\nthree]\nbuiltin-tail: []\n"
    );
    assert!(stderr.is_empty(), "no diagnostics expected: {stderr}");
}

#[test]
fn issue1877_comsub_external_after_dup_from_numbered_fd() {
    // fd 0 bound by dup from a numbered file fd is the same open file
    // description: the substitution reads its head.
    let (stdout, stderr, success) = run_script_watchdog(
        "exec 3<data3.txt\nexec <&3\nx=$(head -n 1)\nprintf 'X=%s\\n' \"$x\"\n",
        Duration::from_secs(20),
    );
    assert!(success, "run must complete, stderr: {stderr}");
    assert_eq!(stdout, "X=one\n");
    assert!(stderr.is_empty(), "no diagnostics expected: {stderr}");
}

#[test]
fn issue1877_comsub_own_redirect_still_wins() {
    // Control: an explicit `<` inside the substitution keeps its own
    // fresh open (the fd-table arms only apply with no local redirect).
    let (stdout, stderr, success) = run_script_watchdog(
        "exec < data3.txt\nread -r a\nx=$(head -n 1 < data3.txt)\nprintf 'own-redirect: %s\\n' \"$x\"\n",
        Duration::from_secs(20),
    );
    assert!(success, "run must complete, stderr: {stderr}");
    assert_eq!(stdout, "own-redirect: one\n");
    assert!(stderr.is_empty(), "no diagnostics expected: {stderr}");
}

#[test]
fn issue1877_exec_closed_stdin_external_child_fails() {
    // Top-level closed fd 0 (oracle-pinned): the external child reads
    // a closed descriptor, fails, and the script continues.
    let (stdout, _stderr, success) = run_script_watchdog(
        "exec <&-\nhead -n 1 2>/dev/null\necho \"head-rc=$?\"\n",
        Duration::from_secs(20),
    );
    assert!(success, "run must complete");
    assert_eq!(stdout, "head-rc=1\n");
}

#[test]
fn issue1877_comsub_closed_stdin_completes_with_failure() {
    // The oracle hangs on this shape (see the module note), so this
    // pin asserts the fork-model completion: EBADF failure inside the
    // substitution, empty capture, script continues, no hang.
    let (stdout, _stderr, success) = run_script_watchdog(
        "exec <&-\nx=$(head -n 1 2>/dev/null)\nprintf 'rc=%s X=[%s]\\n' \"$?\" \"$x\"\n",
        Duration::from_secs(20),
    );
    assert!(success, "run must complete");
    assert_eq!(stdout, "rc=1 X=[]\n");
}
