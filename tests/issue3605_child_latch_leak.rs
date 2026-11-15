//! rubash#3605 (execscript cluster) — per-process latches at the
//! in-process `${THIS_SH}` script-child boundary.
//!
//! `execute_direct_shell_script` emulates GNU's `bash script` fork
//! (execute_cmd.c:6139 shell_execve -> shell.c:429-464
//! shell_reinitialize) on the shared Executor, so every per-process
//! latch the child mutates must be bracketed at the boundary. Two
//! leaked before this fix:
//!
//!   - `exit_trap_ran` (trap.c:1128 run_exit_trap's once-only flag):
//!     a child script's `trap 'echo EXIT' EXIT` fire left the latch
//!     set, so the NEXT in-process child's `( trap T 0; ... )`
//!     subshell found it latched and skipped its own EXIT trap —
//!     execscript's exec3.sub silenced exec12.sub's `exit code: 1`
//!     (and the same leak suppressed the `foo 0` and `WORKS` lines).
//!   - `exec_replaced` (exec.def's replacement unwind): a child's
//!     successful in-process `exec` latched it for the parent, which
//!     suppresses the parent's own EXIT-trap adjudication
//!     (trap_exec's run_exit_trap_for_status early-out).
//!
//! GNU anchors: trap.c:1128 run_exit_trap, builtins/exec.def ->
//! execute_cmd.c:6126 shell_execve, shell.c exit_shell.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3605latch-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn write(dir: &std::path::Path, name: &str, text: &str) {
    fs::write(dir.join(name), text).expect("write script");
}

fn run_c(dir: &std::path::Path, script: &str) -> (String, String, i32) {
    let output = Command::new(RUBASH)
        .args(["-c", script])
        .current_dir(dir)
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

/// A child's EXIT-trap fire must not suppress the next in-process
/// script child's subshell EXIT trap (exec3.sub -> exec12.sub).
#[test]
fn child_exit_trap_fire_does_not_latch_parent_executor() {
    let dir = scratch("exittrap");
    write(
        &dir,
        "p1.sh",
        "trap 'echo E-EXIT' EXIT\necho p1-ran\nexit 0\n",
    );
    write(
        &dir,
        "e12.sh",
        "set -e\nf() { ! :; }\nmain() {( trap 'echo TRAP-FIRED' 0; \"$@\" >>/dev/null 2>&1 )}\nmain f\n",
    );
    let (stdout, _stderr, _rc) = run_c(&dir, "${THIS_SH} p1.sh; ${THIS_SH} e12.sh");
    assert!(
        stdout.contains("p1-ran") && stdout.contains("E-EXIT"),
        "first child should run its EXIT trap, got {stdout:?}"
    );
    assert!(
        stdout.contains("TRAP-FIRED"),
        "second child's subshell EXIT trap must fire — leaked exit_trap_ran suppressed it, got {stdout:?}"
    );
}

/// The latch brackets per child: the SECOND child's own subshell trap
/// fires even when the first child also ran a subshell EXIT trap, and
/// the parent's own `( trap ... 0; ... )` still fires afterwards.
#[test]
fn subshell_exit_traps_survive_across_script_children() {
    let dir = scratch("subtrap");
    write(
        &dir,
        "a.sh",
        "( trap 'echo A-SUB' 0; true )\nexit 0\n",
    );
    write(
        &dir,
        "b.sh",
        "( trap 'echo B-SUB' 0; true )\nexit 0\n",
    );
    let (stdout, _stderr, _rc) = run_c(
        &dir,
        "${THIS_SH} a.sh; ${THIS_SH} b.sh; ( trap 'echo PARENT-SUB' 0; true )",
    );
    for want in ["A-SUB", "B-SUB", "PARENT-SUB"] {
        assert!(stdout.contains(want), "missing {want} in {stdout:?}");
    }
}
