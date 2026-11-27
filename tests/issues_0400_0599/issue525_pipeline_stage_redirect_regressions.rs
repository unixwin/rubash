//! Issue rubash#525 — a pipeline member's INPUT redirect failure must
//! abort that member with its diagnostic and status 1; it must never fall
//! back to the upstream pipe, and the pipeline status must keep counting
//! the surviving members.
//!
//! GNU applies each member's redirections in its own subshell before the
//! command runs (redir.c do_redirections from execute_simple_command), so:
//!
//! ```text
//! echo a | { cat < /nonexistent_zq; }
//! echo rc=$?
//! ```
//!
//! prints the open-failure diagnostic and rc=1 — the member dies, `cat`
//! never reads the pipe. rubash used to drop the redirect on the stage's
//! in-process paths: `{ cat < missing; }` read the upstream payload,
//! printed `a`, and exited 0; a member `read x <$(echo a b)` was routed to
//! the external spawner (`read: command not found`, PIPESTATUS 127)
//! instead of reporting `$(echo a b): ambiguous redirect` with status 1;
//! and a failed mid-member left the whole pipeline status at 1 where GNU
//! keeps the last member's 0.
//!
//! Byte-verified against WSL GNU Bash 5.3.0 (script-file oracle,
//! 2026-10-12).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// #525 repro 1: a brace-group member with an unopenable `< file` dies
/// with the diagnostic; the upstream payload is NOT consumed and the
/// script sees rc=1.
#[test]
fn brace_member_missing_stdin_redirect_dies() {
    let (stdout, stderr, code) = rubash("echo a | { cat < /nonexistent_zq; }\necho rc=$?");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("/nonexistent_zq: No such file or directory"),
        "{stderr:?}"
    );
    assert_eq!(code, Some(0));
}

/// #525: the same for a parenthesized subshell member.
#[test]
fn subshell_member_missing_stdin_redirect_dies() {
    let (stdout, stderr, _) = rubash("echo a | (cat < /nonexistent_zq)\necho rc=$?");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("/nonexistent_zq: No such file or directory"),
        "{stderr:?}"
    );
}

/// #525 repro 2: a member whose fd-0 target expands to multiple fields is
/// an ambiguous redirect — the diagnostic names the RAW word and the
/// member's PIPESTATUS entry is 1 (not 127), with the outer members at 0.
#[test]
fn mid_stage_ambiguous_redirect_reports_and_marks_pipestatus() {
    let (stdout, stderr, _) = rubash("true | read x <$(echo a b) | cat\necho \"${PIPESTATUS[@]}\"");
    assert_eq!(stdout, "0 1 0\n");
    assert!(
        stderr.contains("$(echo a b): ambiguous redirect"),
        "{stderr:?}"
    );
}

/// #525 repro 3: a failed MIDDLE member does not poison the pipeline
/// status — downstream members still run and the status is the last
/// member's (GNU rc=0).
#[test]
fn failed_middle_member_keeps_pipeline_status() {
    let (stdout, stderr, code) = rubash("echo A | cat < /nonexistent | cat\necho rc=$?");
    assert!(
        stderr.contains("/nonexistent: No such file or directory"),
        "{stderr:?}"
    );
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(code, Some(0));
}

/// #525: a member `read` with an unopenable fd-0 file runs as the BUILTIN
/// (its diagnostic and status 1), never as an external `read` child.
#[test]
fn builtin_member_redirect_failure_stays_builtin() {
    let (stdout, stderr, _) = rubash("echo a | read x < /nonexistent_zq\necho rc=$?");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("/nonexistent_zq: No such file or directory"),
        "{stderr:?}"
    );
    assert!(!stderr.contains("command not found"), "{stderr:?}");
}

/// #525: a member `read` with an OPENABLE fd-0 file still reads the file
/// (the redirect wins over the upstream pipe) and succeeds.
#[test]
fn builtin_member_open_stdin_redirect_reads_file() {
    let dir = std::env::temp_dir().join("rubash525_exists.txt");
    std::fs::write(&dir, b"hello\n").expect("write fixture");
    let path = dir.to_string_lossy().replace('\\', "/");
    let (stdout, stderr, code) = rubash(&format!("echo a | read x < {path}\necho rc=$?"));
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// #525: the redirect-failure abort applies to plain non-pipeline
/// commands too — `true < /nonexistent` reports and exits 1 even though
/// `true` never reads stdin (GNU applies the redirection before the
/// command).
#[test]
fn non_reading_command_missing_stdin_redirect_still_fails() {
    let (stdout, stderr, code) = rubash("true < /nonexistent_zq; echo rc=$?");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("/nonexistent_zq: No such file or directory"),
        "{stderr:?}"
    );
    assert_eq!(code, Some(0));
}

/// #525 boundary kept: `2>/dev/null` BEFORE the failing `<` routes the
/// diagnostic into the null device (GNU redir.c:135 redirection_error
/// reports through the fd 2 binding applied so far — issue #250).
#[test]
fn diagnostic_respects_earlier_stderr_redirect() {
    let (stdout, stderr, code) = rubash("cat 2>/dev/null < /nonexistent_zq; echo rc=$?");
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// #525 boundary kept: an openable member redirect wins over the upstream
/// payload for cat too.
#[test]
fn external_member_open_stdin_redirect_reads_file() {
    let dir = std::env::temp_dir().join("rubash525_cat.txt");
    std::fs::write(&dir, b"FILEDATA\n").expect("write fixture");
    let path = dir.to_string_lossy().replace('\\', "/");
    let (stdout, _, code) = rubash(&format!("echo a | cat < {path}\necho rc=$?"));
    assert_eq!(stdout, "FILEDATA\nrc=0\n");
    assert_eq!(code, Some(0));
}
