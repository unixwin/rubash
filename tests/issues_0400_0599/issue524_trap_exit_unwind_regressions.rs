//! Issue rubash#524 — `exit N` inside a trap action must terminate the
//! shell with that status.
//!
//! GNU: builtins/exit.def unwinds to the top level from inside any trap
//! action (the EXITBLTIN jump) — the shell ends with the exit's own status
//! instead of returning to the interrupted flow (execute_cmd.c:5537).
//! rubash's ERR trap runner used to discard the action's execution result,
//! so the script kept running and exited 0.
//!
//! A BARE `exit` inside a trap action resolves to the $? SAVED BEFORE the
//! trap ran (trap.c:367 saves it; builtins/common.c get_exitstat applies
//! it, POSIX interp 1602) — commands inside the action cannot change it.
//! The guard is the trap's recorded function/source nesting level
//! (trap.c:383): for ERR/RETURN the exit must run at that level, while the
//! EXIT trap's guard is open at every depth (the exit path wipes the
//! nesting first, execute_cmd.c:6090; GNU probes 2026-10-12: inside a
//! function called by the EXIT action, `false` then bare `exit` still
//! exits with the pre-trap status, while the same shape under an ERR trap
//! exits with the current $?).
//!
//! Every case is byte-verified against WSL GNU Bash 5.3.0 (script-file
//! oracle, 2026-10-12).

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

/// #524 core: `exit 42` in an ERR trap action terminates with 42; the
/// interrupted script's tail never runs.
#[test]
fn err_trap_exit_terminates_with_own_status() {
    let (stdout, stderr, code) = rubash("trap 'echo ETRAP; exit 42' ERR\nfalse\necho never");
    assert_eq!(stdout, "ETRAP\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(42));
}

/// #524: a bare `exit` in an ERR trap action exits with the pre-trap $?
/// (the failing command's status), even though the action itself ran a
/// successful `echo` first.
#[test]
fn err_trap_bare_exit_uses_pre_trap_status() {
    let (stdout, _, code) = rubash("false\ntrap 'echo E; exit' ERR\nfalse\necho never");
    assert_eq!(stdout, "E\n");
    assert_eq!(code, Some(1));
}

/// #524: the same unwind works inside a function body reached by the ERR
/// trap action (errtrace inherited) — GNU: ETRAP, rc 42, `never` dropped.
#[test]
fn err_trap_exit_inside_function_terminates() {
    let (stdout, _, code) = rubash(
        "set -E\ntrap 'echo ETRAP; exit 42' ERR\nf() { false; echo after-false; }\nf\necho never",
    );
    assert_eq!(stdout, "ETRAP\n");
    assert_eq!(code, Some(42));
}

/// #524 wave2a site: the failing command inside a COMPOUND brace group is
/// an ERR trap firing point like any top-level command, so the action's
/// `exit 9` terminates with 9 and the group's tail never runs (GNU probe:
/// `trap 'echo E; exit 9' ERR; { false; }; echo not-reached` prints E and
/// exits 9).
#[test]
fn err_trap_exit_inside_brace_group_terminates() {
    let (stdout, _, code) = rubash("trap 'echo E; exit 9' ERR; { false; }; echo not-reached");
    assert_eq!(stdout, "E\n");
    assert_eq!(code, Some(9));
    let (stdout, _, code) = rubash("trap 'echo E; exit 9' ERR\n{ false; }\necho not-reached");
    assert_eq!(stdout, "E\n");
    assert_eq!(code, Some(9));
}

/// #524 boundary kept: a failing command in a subshell member of the ERR
/// site — the subshell contains the failure but the action's exit still
/// terminates the parent shell with its own status.
#[test]
fn err_trap_exit_after_subshell_failure_terminates() {
    let (stdout, _, code) = rubash("trap 'echo E; exit 9' ERR\n( false )\necho not-reached");
    assert_eq!(stdout, "E\n");
    assert_eq!(code, Some(9));
}

/// #524: a bare `exit` at the ERR action's own level inside a FUNCTION
/// called by the action uses the current $? (the nesting guard excludes
/// the interp-1602 saved value there; GNU probe: entry $? 1, `echo E`
/// succeeds, bare `exit` -> 0).
#[test]
fn err_trap_function_bare_exit_uses_current_status() {
    let (stdout, _, code) = rubash("trap 'f' ERR\nf() { echo E; exit; }\nfalse\necho never");
    assert_eq!(stdout, "E\n");
    assert_eq!(code, Some(0));
}

/// #524: `exit 42` in a DEBUG trap action terminates with 42 (already
/// propagating; pinned so the ERR fix cannot diverge from it).
#[test]
fn debug_trap_exit_terminates_with_own_status() {
    let (stdout, _, code) = rubash("trap 'echo D; exit 42' DEBUG\necho hi\necho never");
    assert_eq!(stdout, "D\n");
    assert_eq!(code, Some(42));
}

/// #524: `exit 42` in a RETURN trap action (functrace inheritance)
/// terminates with 42.
#[test]
fn return_trap_exit_terminates_with_own_status() {
    let (stdout, _, code) = rubash(
        "set -o functrace\nf() { echo in-f; }\ntrap 'echo R; exit 42' RETURN\nf\necho never",
    );
    assert_eq!(stdout, "in-f\nR\n");
    assert_eq!(code, Some(42));
}

/// #524: a bare `exit` in the EXIT trap action exits with the status the
/// shell was already exiting with — NOT with the action's own last
/// command's status (GNU probe: rc 1, not 0).
#[test]
fn exit_trap_bare_exit_uses_saved_status() {
    let (stdout, _, code) = rubash("trap 'echo T; exit' EXIT\nfalse");
    assert_eq!(stdout, "T\n");
    assert_eq!(code, Some(1));
    let (stdout, _, code) = rubash("trap 'echo T; exit' EXIT\ntrue\necho body");
    assert_eq!(stdout, "body\nT\n");
    assert_eq!(code, Some(0));
}

/// #524: inside a FUNCTION called by the EXIT trap action, a bare `exit`
/// still uses the saved status (the EXIT guard is open at every depth;
/// GNU probe: entry $? 5, `echo T` then bare `exit` -> 5).
#[test]
fn exit_trap_function_bare_exit_uses_saved_status() {
    let (stdout, _, code) = rubash("trap 'g' EXIT\ng() { echo T; exit; }\necho body\nexit 5");
    assert_eq!(stdout, "body\nT\n");
    assert_eq!(code, Some(5));
}

/// #524: an explicit `exit 9` inside a sourced file invoked BY an ERR trap
/// action keeps its own status (GNU probe: rc 9).
#[test]
fn sourced_exit_inside_err_action_keeps_own_status() {
    // temp_dir-based probe path (/tmp is not writable on Android).
    let inner = std::env::temp_dir().join("rubash524_inner.sh");
    let script = format!(
        "echo 'echo INSIDE; exit 9' > '{}'\ntrap '. \"{}\"' ERR\nfalse\necho never",
        inner.to_string_lossy().replace('\\', "/"),
        inner.to_string_lossy().replace('\\', "/")
    );
    let (stdout, _, code) = rubash(&script);
    let _ = std::fs::remove_file(&inner);
    assert_eq!(stdout, "INSIDE\n");
    assert_eq!(code, Some(9));
}

/// #524 boundary kept: an EXIT trap action that does NOT exit leaves the
/// shell's exit status untouched (the action's own success does not
/// overwrite it).
#[test]
fn exit_trap_without_exit_keeps_status() {
    let (stdout, _, code) = rubash("trap 'echo cleanup' EXIT\nexit 7");
    assert_eq!(stdout, "cleanup\n");
    assert_eq!(code, Some(7));
    let (_, _, code) = rubash("trap 'false' EXIT\nexit 7");
    assert_eq!(code, Some(7));
}
