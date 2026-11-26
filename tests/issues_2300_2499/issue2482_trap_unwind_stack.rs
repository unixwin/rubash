//! rubash#2482 (face a): a fatal unwind out of nested functions —
//! the errexit exit (execute_cmd.c exit_immediately_on_error) and
//! the word-expansion DISCARD under armed errexit (subst.c:4296 /
//! subst.c:13104 exp_jump_to_top_level -> sig.c:490
//! jump_to_top_level) — longjmps straight past execute_function's
//! frame pop, so the EXIT trap action executes with the dying call
//! stack still installed: FUNCNAME/BASH_LINENO/BASH_SOURCE name the
//! failed function, and the action's first command takes a DEBUG
//! re-presentation firing wearing the frozen BASH_COMMAND with its
//! caller-line slot reset to 1 (parse_and_execute's SEVAL_RESETLINE
//! for the trap string). bats' tracing ring (bats.bats 55/57/59/64)
//! reads exactly that stack via the $BASH_SOURCE trap argument.
//!
//! Expectations below are GNU bash 5.3.20 outputs, probed from
//! script files on 2026-11-02. The no-errexit DISCARD case is the
//! counter-shape: sig.c:387 top_level_cleanup runs the unwind
//! protects, the frames pop, and the EXIT trap sees an empty stack.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn rubash_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .env_remove("WINUXSH_ROOT")
        .env_remove("__RUBASH_SHELL_ROOT");
    command
}

fn shell_test_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2482-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn run_script(dir: &Path, name: &str, body: &str) -> std::process::Output {
    let script = dir.join(name);
    fs::write(&script, body).expect("write script");
    rubash_command()
        .current_dir(dir)
        .arg(shell_test_path(&script))
        .output()
        .expect("run script")
}

// 01 — errexit failure (`false`) inside a function nested two deep:
// the ERR trap fires once with the full stack, and the EXIT trap
// action still sees the dying stack (GNU: teardown_trap inner_fn
// perform main, lines 1 12 14 0 in the probe's file layout).
#[test]
fn issue2482_01_errexit_exit_trap_sees_dying_stack() {
    let dir = test_dir("01");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eET\n\
         exec 3>&1\n\
         errfn() { echo \"ERRFN fn=${FUNCNAME[*]}\" >&3; }\n\
         teardown_trap() { echo \"EXITTRAP fn=${FUNCNAME[*]} ln=${BASH_LINENO[*]}\" >&3; }\n\
         trap 'errfn' ERR\n\
         trap 'teardown_trap' EXIT\n\
         inner_fn() {\n  false\n}\n\
         perform() {\n  inner_fn\n}\n\
         perform\n",
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "ERRFN fn=errfn inner_fn perform main\n\
         EXITTRAP fn=teardown_trap inner_fn perform main ln=1 11 13 0\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

// 02 — word-expansion DISCARD (unbound variable) under armed
// errexit: same dying-stack visibility for the EXIT trap, this
// time with a BASH_SOURCE argument check (the bats exclusion
// input): inside the re-presentation DEBUG firing, $BASH_SOURCE
// names the failed function's file, not the main script.
#[test]
fn issue2482_02_discard_exit_trap_sees_dying_stack() {
    let dir = test_dir("02");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eET\n\
         exec 3>&1\n\
         teardown_trap() { echo \"EXITTRAP fn=${FUNCNAME[*]} ln=${BASH_LINENO[*]}\" >&3; }\n\
         trap 'teardown_trap' EXIT\n\
         inner_fn() {\n  set -u\n  echo \"$unset_parameter\"\n}\n\
         perform() {\n  inner_fn\n}\n\
         perform\n",
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "EXITTRAP fn=teardown_trap inner_fn perform main ln=1 10 12 0\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

// 03 — the EXIT action as a bare string: the ${FUNCNAME[*]}
// expansion inside the action itself (no called function) reads
// the leaked stack — FUNCNAME's att_invisible visibility
// (variables.c:1812 make_funcname_visible) is cleared on the
// epilogue path the longjmp skips, so the array stays visible at
// depth 0 while the leaked frames remain.
#[test]
fn issue2482_03_discard_string_exit_action_reads_stack() {
    let dir = test_dir("03");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eET\n\
         exec 3>&1\n\
         trap 'echo \"EXITTRAP fn=${FUNCNAME[*]}\" >&3' EXIT\n\
         inner_fn() {\n  set -u\n  echo \"$unset_parameter\"\n}\n\
         perform() {\n  inner_fn\n}\n\
         perform\n",
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "EXITTRAP fn=inner_fn perform main\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

// 04 — counter-shape: the same DISCARD with errexit OFF unwinds
// through top_level_cleanup (sig.c:387), the frames pop, and the
// EXIT trap sees an empty FUNCNAME. The whole run still aborts
// with status 1 in both shells (nothing after the call runs).
#[test]
fn issue2482_04_discard_without_errexit_stack_is_clean() {
    let dir = test_dir("04");
    let output = run_script(
        &dir,
        "case.sh",
        "set -ET\n\
         exec 3>&1\n\
         trap 'echo \"EXITTRAP fn=${FUNCNAME[*]}\" >&3' EXIT\n\
         inner_fn() {\n  set -u\n  echo \"$unset_parameter\"\n}\n\
         perform() {\n  inner_fn\n  echo \"perform continues\" >&3\n}\n\
         perform\n\
         echo \"main continues\" >&3\n",
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "EXITTRAP fn=\n");
    let _ = fs::remove_dir_all(&dir);
}

// 05 — the DEBUG re-presentation sequence across the fatal
// unwind: after the failing command's own firing, the EXIT
// action's first command takes one more DEBUG firing wearing the
// frozen BASH_COMMAND with the caller frame back at main and the
// line slot reset to 1 — the rotation bats' LAST/LASTLAST ring
// depends on. The full firing sequence is pinned.
#[test]
fn issue2482_05_debug_representation_sequence() {
    let dir = test_dir("05");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eET\n\
         exec 3>&1\n\
         dbg() { echo \"DBG cmd=[$BASH_COMMAND] fn=${FUNCNAME[*]} ln=${BASH_LINENO[*]}\" >&3; }\n\
         trap 'dbg' DEBUG\n\
         trap 'echo \"EXITTRAP\" >&3' EXIT\n\
         inner_fn() {\n  set -u\n  echo \"$unset_parameter\"\n}\n\
         perform() {\n  inner_fn\n}\n\
         perform\n",
    );
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let dbg_lines: Vec<&str> = stdout.lines().filter(|l| l.starts_with("DBG")).collect();
    assert_eq!(
        dbg_lines,
        [
            "DBG cmd=[trap 'echo \"EXITTRAP\" >&3' EXIT] fn=dbg main ln=5 0",
            "DBG cmd=[perform] fn=dbg main ln=13 0",
            "DBG cmd=[perform] fn=dbg perform main ln=10 13 0",
            "DBG cmd=[inner_fn] fn=dbg perform main ln=11 13 0",
            "DBG cmd=[inner_fn] fn=dbg inner_fn perform main ln=6 11 13 0",
            "DBG cmd=[set -u] fn=dbg inner_fn perform main ln=7 11 13 0",
            "DBG cmd=[echo \"$unset_parameter\"] fn=dbg inner_fn perform main ln=8 11 13 0",
            "DBG cmd=[echo \"$unset_parameter\"] fn=dbg inner_fn perform main ln=1 11 13 0",
        ],
        "stdout: {stdout}"
    );
    assert!(stdout.ends_with("EXITTRAP\n"), "stdout: {stdout}");
    let _ = fs::remove_dir_all(&dir);
}

// 07 — the DISCARD under a `||` suppression still leaks every
// frame and still aborts the run: suppression is a per-command
// ignore_return flag and does NOT clear GNU's
// exit_immediately_on_error (flags.c:263), so the gate is the
// live `-e` flag at the failure point, not the suppression
// state. Neither "survived" line prints in either shell.
#[test]
fn issue2482_07_discard_under_suppression_still_leaks() {
    let dir = test_dir("07");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eET\n\
         exec 3>&1\n\
         trap 'echo \"EXITTRAP fn=${FUNCNAME[*]}\" >&3' EXIT\n\
         inner_fn() {\n  set -u\n  echo \"$unset_parameter\"\n}\n\
         perform() {\n  inner_fn || st=$?\n  echo \"perform survived\" >&3\n}\n\
         perform\n\
         echo \"main survived\" >&3\n",
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "EXITTRAP fn=inner_fn perform main\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

// 08 — the gate follows the live flag at the failure point:
// `-e` armed only inside the dying function leaks; `-e` armed
// globally but turned off inside the dying function does not.
#[test]
fn issue2482_08_errexit_scope_at_failure_point() {
    let dir = test_dir("08a");
    let output = run_script(
        &dir,
        "case.sh",
        "set -ET\n\
         exec 3>&1\n\
         trap 'echo \"EXITTRAP fn=${FUNCNAME[*]}\" >&3' EXIT\n\
         inner_fn() {\n  set -eu\n  echo \"$unset_parameter\"\n}\n\
         perform() { inner_fn; }\n\
         perform\n",
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "EXITTRAP fn=inner_fn perform main\n"
    );
    let _ = fs::remove_dir_all(&dir);

    let dir = test_dir("08b");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eET\n\
         exec 3>&1\n\
         trap 'echo \"EXITTRAP fn=${FUNCNAME[*]}\" >&3' EXIT\n\
         inner_fn() {\n  set +e\n  set -u\n  echo \"$unset_parameter\"\n}\n\
         perform() { inner_fn; }\n\
         perform\n",
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "EXITTRAP fn=\n");
    let _ = fs::remove_dir_all(&dir);
}

// 06 — `exit` inside a nested function: GNU runs the EXIT trap
// with the call stack in place (the builtin's jump_to_top_level
// EXITPROG skips the pops the same way); rubash runs the action
// at the dispatch site with the frames in place — same visible
// stack, same status.
#[test]
fn issue2482_06_exit_builtin_stack_visible() {
    let dir = test_dir("06");
    let output = run_script(
        &dir,
        "case.sh",
        "set -ET\n\
         exec 3>&1\n\
         teardown_trap() { echo \"EXITTRAP fn=${FUNCNAME[*]} ln=${BASH_LINENO[*]}\" >&3; }\n\
         trap 'teardown_trap' EXIT\n\
         inner_fn() {\n  exit 3\n}\n\
         perform() {\n  inner_fn\n}\n\
         perform\n\
         echo \"unreachable\" >&3\n",
    );
    assert_eq!(output.status.code(), Some(3));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "EXITTRAP fn=teardown_trap inner_fn perform main ln=1 9 11 0\n"
    );
    let _ = fs::remove_dir_all(&dir);
}
