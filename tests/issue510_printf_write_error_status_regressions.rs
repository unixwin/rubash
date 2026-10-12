//! Issue rubash#510 regression: a builtin write failure must reach the exit
//! status — `exec >&-; printf "end\n"` reports the GNU diagnostic but exited
//! 0 where GNU exits 1 (builtins/printf.def printf_builtin -> sh_chkwrite
//! folds the failed flush into the builtin's status, builtins/common.c
//! 320-334).
//!
//! GNU spec (WSL GNU bash 5.3.0 probes, 2026-10-12, all stdin-driven):
//!
//! ```text
//! $ printf 'exec >&-; printf "end\n"' | bash -s; echo $?
//! bash: line 1: printf: write error: Bad file descriptor
//! 1
//! $ printf 'exec >&-; printf x || exit 7' | bash -s; echo $?
//! bash: line 1: printf: write error: Bad file descriptor
//! 7
//! $ printf 'exec >&-; printf ""' | bash -s; echo $?
//! 0                        # no output, no write, no failure
//! ```
//!
//! Rust semantic owner: executor/printf_path_builtins.rs execute_printf —
//! the buffered-output router already printed the diagnostic and raised the
//! executor's last_builtin_write_failed flag; the wrapper now merges it into
//! the returned status (the same merge execute_pwd applies) so the dispatch
//! site's `self.exit_code = self.execute_printf(cmd)?` keeps the failure.

#![cfg(windows)]

use std::io::Write;
use std::process::{Command, Stdio};

fn run_stdin(script: &str) -> (Option<i32>, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(script.as_bytes())
        .expect("write script to stdin");
    let output = child.wait_with_output().expect("wait for rubash");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The issue repro: the diagnostic was already right; the status must be 1.
#[test]
fn issue_repro_printf_write_error_exits_1() {
    let (code, _stdout, stderr) = run_stdin("exec >&-; printf \"end\\n\"\n");
    assert_eq!(code, Some(1));
    assert!(
        stderr.contains("printf: write error: Bad file descriptor"),
        "missing write-error diagnostic: {stderr:?}"
    );
}

/// The status is a real command status: `||` sees the failure (GNU rc 7).
#[test]
fn write_failure_drives_the_or_handler() {
    let (code, _stdout, _stderr) = run_stdin("exec >&-; printf x || exit 7\n");
    assert_eq!(code, Some(7));
}

/// errexit promotes the write failure like any other command failure (GNU
/// rc 1, the shell stops).
#[test]
fn write_failure_honors_errexit() {
    let (code, _stdout, _stderr) = run_stdin("set -e; exec >&-; printf x; echo UNREACHED\n");
    assert_eq!(code, Some(1));
}

/// No output means no write, so no failure (GNU rc 0) — the merge must not
/// turn a silent printf into a failure.
#[test]
fn empty_output_printf_still_succeeds_on_closed_stdout() {
    let (code, _stdout, _stderr) = run_stdin("exec >&-; printf \"\"\n");
    assert_eq!(code, Some(0));
}

/// printf's ordinary statuses are untouched: a real format run to a working
/// stdout succeeds, a format failure still fails.
#[test]
fn ordinary_printf_statuses_unchanged() {
    let dir = std::env::temp_dir().join(format!("rubash-issue510-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue510 scratch dir");
    let out_path = dir.join("out.txt");
    let script = format!(
        "printf 'a%sb\\n' X > {}; echo done\n",
        out_path.to_string_lossy().replace('\\', "/")
    );
    let (code, stdout, _stderr) = run_stdin(&script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "done\n");
    assert_eq!(
        std::fs::read_to_string(&out_path).unwrap_or_default(),
        "aXb\n"
    );
    let _ = std::fs::remove_dir_all(&dir);

    let (code, _stdout, _stderr) = run_stdin("printf '%d' novalue 2>/dev/null\n");
    assert_eq!(code, Some(1));
}
