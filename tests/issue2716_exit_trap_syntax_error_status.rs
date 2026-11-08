//! EXIT trap action with a syntax error (rubash#2716; GNU bash
//! 5.3.20 oracle-verified 2026-11-09).
//!
//! GNU `run_exit_trap` (trap.c:1027) saves the pending status in
//! `trap_saved_exit_value` (trap.c:1036) and runs the action through
//! `parse_and_execute (trap_command, "exit trap", ...)` (trap.c:1071):
//! the string name becomes the diagnostic context, so parse errors
//! read `<prog>: exit trap: line 1: syntax error ...` plus a source
//! echo line, and a hard failure of the action — including a parse
//! failure — does NOT replace the saved exit status (only an explicit
//! exit jump out of the action does). Rubash's
//! `run_exit_trap_action` (src/executor/trap_exec.rs) re-parsed the
//! action without a trap context name and propagated the re-parse
//! error out of the exit funnel, so the shell exited 2 with
//! context-free diagnostics.
//!
//! Script-file pins; the script path in diagnostics is normalized
//! to `S`. Diagnostics are asserted on their stable parts (the
//! `exit trap:` infix and the message class) plus exact stdout and
//! exact exit status.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn rubash_bin() -> String {
    env!("CARGO_BIN_EXE_rubash").to_string()
}

/// Run a script file; return (normalized stdout, normalized stderr, code).
fn run_script(tag: &str, body: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("issue2716-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    let path: PathBuf = dir.join("case.sh");
    fs::write(&path, body).expect("write script");
    let output = Command::new(rubash_bin())
        .arg(&path)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("run rubash");
    let normalize = |bytes: &[u8]| {
        String::from_utf8_lossy(bytes)
            .replace(path.to_str().expect("utf8 path"), "S")
            .into_owned()
    };
    (
        normalize(&output.stdout),
        normalize(&output.stderr),
        output.status.code(),
    )
}

/// GNU's two diagnostic lines for the `echo (` action, normalized:
/// the syntax-error line and the source echo line, both carrying the
/// `exit trap:` infix.
fn assert_exit_trap_diagnostics(stderr: &str) {
    assert!(
        stderr.contains("S: exit trap: line 1: syntax error near unexpected token"),
        "diagnostics must carry the exit trap context: {stderr:?}"
    );
    assert!(
        stderr.contains("S: exit trap: line 1: `echo ('"),
        "source echo line must carry the exit trap context: {stderr:?}"
    );
}

#[test]
fn issue2716_canonical_syntax_error_preserves_zero_status() {
    // The issue's canonical shape.
    let (stdout, stderr, code) = run_script(
        "canon",
        "trap 'echo (' EXIT\necho \"defined ok rc=$?\"\necho last\n",
    );
    assert_eq!(stdout, "defined ok rc=0\nlast\n");
    assert_eq!(code, Some(0));
    assert_exit_trap_diagnostics(&stderr);
}

#[test]
fn issue2716_explicit_exit_status_survives_trap_syntax_error() {
    let (stdout, stderr, code) = run_script("exit9", "trap 'echo (' EXIT\nexit 9\n");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(9));
    assert_exit_trap_diagnostics(&stderr);
}

#[test]
fn issue2716_pending_failure_status_survives_trap_syntax_error() {
    // The pending status is the last command's (false -> 1), not the
    // trap re-parse's 2.
    let (_stdout, stderr, code) = run_script("pending1", "trap 'echo (' EXIT\nfalse\n");
    assert_eq!(code, Some(1));
    assert_exit_trap_diagnostics(&stderr);
}

#[test]
fn issue2716_subshell_exit_status_survives_trap_syntax_error() {
    // The funnel is shared by subshell exits: the subshell's own
    // pending status (3) is what the parent must observe.
    let (stdout, stderr, code) = run_script(
        "subshell",
        "( trap 'echo (' EXIT; exit 3 )\necho \"sub_rc=$?\"\n",
    );
    assert_eq!(stdout, "sub_rc=3\n");
    assert_eq!(code, Some(0));
    assert_exit_trap_diagnostics(&stderr);
}

#[test]
fn issue2716_status_from_last_compound_survives_trap_syntax_error() {
    // Chain: the pending status is produced by a compound command
    // after the trap is armed; the trap's parse failure must not
    // overwrite it.
    let (_stdout, stderr, code) =
        run_script("chain", "trap 'echo (' EXIT\n( exit 6 )\n");
    assert_eq!(code, Some(6));
    assert_exit_trap_diagnostics(&stderr);
}

#[test]
fn issue2716_failing_trap_action_still_preserves_status() {
    // Regression lock (already correct per the issue): an action
    // whose COMMANDS fail is not a hard error — status preserved,
    // no diagnostics.
    let (stdout, stderr, code) = run_script("false-action", "trap 'false' EXIT\necho hi\n");
    assert_eq!(stdout, "hi\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue2716_valid_trap_action_still_runs_and_preserves_status() {
    // Regression lock: a well-formed action runs and the pending
    // status passes through.
    let (stdout, stderr, code) = run_script("valid", "trap 'echo bye' EXIT\necho hi\n");
    assert_eq!(stdout, "hi\nbye\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
