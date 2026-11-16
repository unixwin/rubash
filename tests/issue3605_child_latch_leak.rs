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
//!
//! Pin mechanics: the leak only exists on a SHARED Executor, so the
//! children must run in-process. The rubash binary opts into real
//! shell children (main.rs enable_real_shell_children), which makes
//! `${THIS_SH} x.sh` spawn a fresh process per child — a pin driven
//! through `Command::new(rubash)` can never observe the latch. This
//! pin therefore drives `Executor` directly through the library:
//! `real_shell_children_enabled()` stays false, `execute_same_shell_script`
//! takes the in-process `execute_direct_shell_script` branch
//! (external_finish.rs), and both children share one Executor. Child
//! output is asserted through an events file because `execute_ast`
//! writes to the real process stdout, which a test cannot capture.

use rubash::executor::Executor;
use rubash::lexer::tokenize;
use rubash::parser::parse;
use std::fs;
use std::path::{Path, PathBuf};

fn scratch(tag: &str) -> PathBuf {
    let dir =
        PathBuf::from("target").join(format!("rubash-3605latch-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Forward-slash relative spelling for shell source: Rust's `Path`
/// display emits `\` on Windows, which the shell lexer would read as
/// escapes.
fn shell_path(dir: &Path, name: &str) -> String {
    dir.join(name).to_string_lossy().replace('\\', "/")
}

fn write(dir: &Path, name: &str, text: &str) {
    fs::write(dir.join(name), text).expect("write script");
}

fn run_source(executor: &mut Executor, source: &str) {
    let tokens = tokenize(source);
    let ast = parse(&tokens);
    let _ = executor.execute_ast(&ast);
}

fn read_events(dir: &Path) -> String {
    fs::read_to_string(dir.join("events.log")).unwrap_or_default()
}

/// A child's EXIT-trap fire must not suppress the next in-process
/// script child's subshell EXIT trap (exec3.sub -> exec12.sub).
#[test]
fn child_exit_trap_fire_does_not_latch_parent_executor() {
    let dir = scratch("exittrap");
    let events_path = shell_path(&dir, "events.log");
    write(
        &dir,
        "p1.sh",
        &format!(
            "trap 'echo P1-EXIT >> {events_path}' EXIT\n\
             echo P1-RAN >> {events_path}\n\
             exit 0\n"
        ),
    );
    write(
        &dir,
        "e12.sh",
        &format!(
            "set -e\n\
             f() {{ ! :; }}\n\
             main() {{( trap 'echo TRAP-FIRED >> {events_path}' 0; \"$@\" >>/dev/null 2>&1 )}}\n\
             main f\n"
        ),
    );
    let mut executor = Executor::new();
    run_source(
        &mut executor,
        &format!(
            "${{THIS_SH}} {} ; ${{THIS_SH}} {}",
            shell_path(&dir, "p1.sh"),
            shell_path(&dir, "e12.sh")
        ),
    );
    let log = read_events(&dir);
    assert!(
        log.contains("P1-RAN") && log.contains("P1-EXIT"),
        "first child should run its EXIT trap, got {log:?}"
    );
    assert!(
        log.contains("TRAP-FIRED"),
        "second child's subshell EXIT trap must fire — leaked exit_trap_ran suppressed it, got {log:?}"
    );
}

/// The latch brackets per child: the SECOND child's own subshell trap
/// fires even when the first child also ran a subshell EXIT trap, and
/// the parent's own `( trap ... 0; ... )` still fires afterwards.
#[test]
fn subshell_exit_traps_survive_across_script_children() {
    let dir = scratch("subtrap");
    let events_path = shell_path(&dir, "events.log");
    write(
        &dir,
        "a.sh",
        &format!("( trap 'echo A-SUB >> {events_path}' 0; true )\nexit 0\n"),
    );
    write(
        &dir,
        "b.sh",
        &format!("( trap 'echo B-SUB >> {events_path}' 0; true )\nexit 0\n"),
    );
    let mut executor = Executor::new();
    run_source(
        &mut executor,
        &format!(
            "${{THIS_SH}} {} ; ${{THIS_SH}} {} ; ( trap 'echo PARENT-SUB >> {events_path}' 0; true )",
            shell_path(&dir, "a.sh"),
            shell_path(&dir, "b.sh")
        ),
    );
    let log = read_events(&dir);
    for want in ["A-SUB", "B-SUB", "PARENT-SUB"] {
        assert!(log.contains(want), "missing {want} in {log:?}");
    }
}
