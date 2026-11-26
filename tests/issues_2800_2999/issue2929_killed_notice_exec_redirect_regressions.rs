//! rubash#2929: the Killed job-death notice bypasses an `exec 2>`
//! redirect.
//!
//! GNU ground truth (bash 5.3.20): `exec 2> file` dup2s the process
//! fd 2 in the shell (redir.c), so the async-reap notice — written to
//! the shell's stderr — lands in the file. Rubash's `exec 2>` only
//! records the redirect in the shell's fd table, which child
//! commands and builtin stderr writers consult; the notice writer
//! (`write_stderr_bytes`, job_builtins.rs `report_pending_signal_notices`)
//! writes the raw process fd 2 and bypasses the table, so the notice
//! escapes to the outer stderr while the file stays empty.
//!
//! Fix: route the notice through the shell's current fd-2 binding
//! (`Executor::write_diagnostic_fd2`, shell_options.rs — rubash#167).
//! The `wait`-scoped `2>/dev/null` face (#1303) stays untouched: a
//! per-command redirect must NOT divert the shell-level notice.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_script_file(
    body: &str,
    dir_name: &str,
) -> (String, String, Option<i32>, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("rubash-2929-{dir_name}-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("seed.sh");
    {
        let mut file = std::fs::File::create(&path).expect("write script");
        file.write_all(body.as_bytes()).expect("write script");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
        dir,
    )
}

/// The issue seed: `exec 2> reap.err`, async KILL — the notice must
/// land in reap.err and the outer stderr must stay empty.
#[test]
fn killed_notice_follows_exec_stderr_redirect() {
    let (stdout, stderr, rc, dir) = run_script_file(
        "exec 2> reap.err\nsleep 3 & p=$!\nkill -KILL $p\nsleep 0.5\necho done\n",
        "exec-redirect",
    );
    assert_eq!(rc, Some(0));
    assert!(stdout.contains("done"), "seed must complete: {stdout:?}");
    assert!(
        stderr.is_empty(),
        "outer stderr must stay empty once 2> is in force: {stderr:?}"
    );
    let reap = std::fs::read_to_string(dir.join("reap.err")).expect("read reap.err");
    assert!(
        reap.contains("Killed"),
        "the notice must land in reap.err: {reap:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// `exec 2>&1`: the notice follows fd 2 onto stdout.
#[test]
fn killed_notice_follows_exec_stderr_to_stdout() {
    let (stdout, stderr, rc, dir) = run_script_file(
        "exec 2>&1\nsleep 3 & p=$!\nkill -KILL $p\nsleep 0.5\necho done\n",
        "exec-stdout",
    );
    assert_eq!(rc, Some(0));
    assert!(
        stderr.is_empty(),
        "outer stderr must stay empty once 2>&1 is in force: {stderr:?}"
    );
    assert!(
        stdout.contains("Killed"),
        "the notice must follow fd 2 onto stdout: {stdout:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// `exec 2>&-` (closed stderr): the notice is dropped silently —
/// GNU ignores the failed write — and the run completes.
#[test]
fn killed_notice_dropped_silently_on_closed_stderr() {
    let (stdout, stderr, rc, dir) = run_script_file(
        "exec 2>&-\nsleep 3 & p=$!\nkill -KILL $p\nsleep 0.5\necho done\n",
        "exec-closed",
    );
    assert_eq!(rc, Some(0));
    assert!(stdout.contains("done"), "seed must complete: {stdout:?}");
    assert!(
        stderr.is_empty(),
        "nothing may escape to the outer stderr: {stderr:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Positive control (#1303): when the reap happened BEFORE the wait
/// (the `sleep 0.5` lets the death land), the shell-level notice is
/// produced by the reader-boundary flush — not by the wait — so the
/// wait's own `2>/dev/null` must NOT divert it. (When the wait
/// itself reaps, GNU also swallows the notice into /dev/null — that
/// face is the wait's own binding, not this issue.)
#[test]
fn per_command_stderr_redirect_does_not_divert_shell_notice() {
    let (stdout, stderr, rc, dir) = run_script_file(
        "sleep 3 & p=$!\nkill -KILL $p\nsleep 0.5\nwait $p 2>/dev/null\necho done\n",
        "wait-redirect",
    );
    assert_eq!(rc, Some(0));
    assert!(stdout.contains("done"), "seed must complete: {stdout:?}");
    assert!(
        stderr.contains("Killed"),
        "the shell-level notice must still reach the shell stderr: {stderr:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
