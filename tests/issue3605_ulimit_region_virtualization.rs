//! rubash#3605 (run-vredir): a `ulimit` executed inside an in-process
//! GNU-fork region — a `( )`/`$( )` layer, a pipeline element, or an
//! in-process `${THIS_SH}`/script child — must not move the real
//! process rlimits. GNU runs those bodies in real children whose
//! setrlimit results die with the process; when the region is modeled
//! in-process, a hard-limit drop is irreversible without
//! CAP_SYS_RESOURCE and leaks into the parent. The builtin therefore
//! virtualizes: SOFT reaches the kernel (always restorable) and the
//! effective pair is recorded in `__RUBASH_ULIMIT_*` env cells for
//! reporting — the same contract the Windows platform layer always
//! runs under.
//!
//! GNU anchor: execute_cmd.c:1576 execute_in_subshell — the child's
//! setrlimit (builtins/ulimit.def:632 set_limit) is private to the
//! forked image and never propagates.

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

/// A bare `( )` layer is a fork region too (execute_cmd.c:1576): the
/// child's hard drop must die with it.
#[test]
fn subshell_hard_drop_does_not_leak() {
    let (stdout, stderr, _) =
        rubash("( ulimit -n 6 )\nulimit -n\nexec {v}</dev/null && echo \"fd=$v\"");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "stdout: {stdout}");
    assert!(
        lines[0].trim().parse::<u64>().unwrap_or(0) > 1000,
        "parent NOFILE leaked the subshell drop: {stdout}"
    );
    assert_eq!(lines[1], "fd=10", "fd alloc after subshell: {stdout}");
}

/// A pipeline element is a GNU fork child (execute_cmd.c:2620
/// execute_pipeline): `ulimit -n 6 | cat` inside must not re-limit
/// the parent process.
#[test]
fn pipeline_element_hard_drop_does_not_leak() {
    let (stdout, stderr, _) =
        rubash("ulimit -n 6 | cat\nulimit -n\nexec {v}</dev/null && echo \"fd=$v\"");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "stdout: {stdout}");
    assert!(
        lines[0].trim().parse::<u64>().unwrap_or(0) > 1000,
        "parent NOFILE leaked the element drop: {stdout}"
    );
    assert_eq!(lines[1], "fd=10", "fd alloc after pipeline: {stdout}");
}

/// Inside the region the set still takes effect for the child itself:
/// `ulimit -n` reports the new pair (from the env cell), and a `{fd}`
/// allocation under a dropped limit sees the EINVAL GNU's real child
/// would see from F_DUPFD.
#[test]
fn region_sees_its_own_effective_limit() {
    let (stdout, stderr, _) =
        rubash("( ulimit -n 6; ulimit -n; exec {v}</dev/null 2>/dev/null; echo \"rc=$?\" )");
    // GNU prints the pair of redirect diagnostics — the `2>/dev/null`
    // entry never applies because the `{v}` entry fails first.
    assert!(
        stderr.contains("redirection error: cannot duplicate fd"),
        "missing sys_error line: {stderr}"
    );
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "stdout: {stdout}");
    assert_eq!(lines[0], "6", "region's own report: {stdout}");
    assert_eq!(lines[1], "rc=1", "region fd alloc under limit 6: {stdout}");
}

/// A function body's `ulimit` shares the CURRENT shell's limits in
/// BOTH shells (GNU: functions are not forked) — the region
/// containment must not swallow it.
#[test]
fn function_ulimit_still_reaches_the_shell() {
    let (stdout, stderr, _) = rubash("f() { ulimit -Sn 7; }; f\nulimit -Sn");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(stdout.trim(), "7", "function set must share: {stdout}");
}
