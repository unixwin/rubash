//! Issue rubash#3219: the flat fork-region `ulimit` model
//! (rubash#2636/#3605) records a region's sets in the
//! `__RUBASH_ULIMIT_*` env-cell overlay and moves only the SOFT side
//! of the shared process, so the parent's read-back matches GNU —
//! but a real child born inside the region inherited the PROCESS
//! pair: its hard side was the parent's, where GNU's forked child
//! carries the region's pair.
//!
//! GNU semantics: a subshell / comsub / pipeline element / coproc is
//! a real fork (execute_cmd.c:1576 execute_in_subshell; subst.c:7143
//! command_substitute; execute_cmd.c:2620 execute_pipeline;
//! execute_cmd.c execute_coproc via make_child), the `ulimit`
//! builtin drives that process's rlimits (builtins/ulimit.def
//! set_limit -> setrlimit, ulimit.def:632), and everything the child
//! spawns — external commands, `exec`'s image (execute_cmd.c:6126
//! shell_execve) — simply inherits the pair across fork/execve.
//!
//! Fix: every spawn site that can birth a child from inside a region
//! applies the recorded cell pairs between fork and exec
//! (arm_emulated_limits / apply_emulated_limits_in_child): the
//! std-Command external funnels, the raw-fork async path, pipeline
//! stages, process substitution, the exec builtin's replacement and
//! simulation constructors, coproc, and the whitelisted child-shell
//! spawn (whose pairs ride the composed spawn env). Outside a region
//! there are no cells and every arm is a no-op.
//!
//! Expected outputs are byte-verified against GNU Bash 5.3.20
//! (self-compiled oracle, script-file probes p1-p13, 2026-11-20):
//! on master every child cell below reads the ambient hard limit
//! instead of 777; with the fix all 21 probes match the oracle.
//! Unix-only: Windows has no process rlimits (the emulated table
//! lives in the env map and rides the state save/restore).

#![cfg(unix)]

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

fn assert_clean(stderr: &str) {
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
}

/// The shell binary itself as an external child: a nested rubash at
/// its top level reads its real (kernel) rlimits, so its read-back
/// is the pair the spawn actually delivered.
fn child_limits_script() -> String {
    format!(
        "\"{}\" -c 'echo \"S=$(ulimit -Sn) H=$(ulimit -Hn)\"'",
        env!("CARGO_BIN_EXE_rubash")
    )
}

/// rubash#3219 criterion (a): the u5 shape — after a both-form set
/// inside a subshell, the parent's soft/hard read-back is untouched
/// (777/777 in the parent was the original divergence; GNU reads
/// back the ambient pair).
#[test]
fn parent_readback_survives_both_form_subshell_set() {
    let (stdout, stderr, status) = rubash(
        "echo \"pre S=$(ulimit -Sn) H=$(ulimit -Hn)\"; \
         ( ulimit -n 777; echo \"in S=$(ulimit -Sn) H=$(ulimit -Hn)\" ); \
         echo \"post S=$(ulimit -Sn) H=$(ulimit -Hn)\"",
    );
    assert_clean(&stderr);
    assert_eq!(status, Some(0));
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3, "stdout: {stdout}");
    assert!(lines[1].ends_with("S=777 H=777"), "in-region: {}", lines[1]);
    let pre = lines[0].strip_prefix("pre ").unwrap();
    let post = lines[2].strip_prefix("post ").unwrap();
    assert_eq!(pre, post, "parent read-back moved: {pre} vs {post}");
    assert!(!pre.contains("777"), "ambient unexpectedly 777: {pre}");
}

/// An external child spawned inside the region is born with the
/// region's pair — soft AND hard (master: hard stayed ambient).
#[test]
fn external_child_in_subshell_sees_region_pair() {
    let script = format!("( ulimit -n 777; {} )", child_limits_script());
    let (stdout, stderr, status) = rubash(&script);
    assert_clean(&stderr);
    assert_eq!(status, Some(0));
    assert_eq!(stdout.trim(), "S=777 H=777", "stdout: {stdout}");
}

/// A soft-only region set delivers soft=777 with the AMBIENT hard —
/// the arm must not invent a hard side the region never set.
#[test]
fn external_child_soft_only_keeps_ambient_hard() {
    let script = format!(
        "echo \"amb H=$(ulimit -Hn)\"; ( ulimit -Sn 777; {} )",
        child_limits_script()
    );
    let (stdout, stderr, status) = rubash(&script);
    assert_clean(&stderr);
    assert_eq!(status, Some(0));
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "stdout: {stdout}");
    let ambient_hard = lines[0].strip_prefix("amb H=").unwrap();
    assert_eq!(lines[1], format!("S=777 H={ambient_hard}"));
}

/// The two-step shape: soft set and hard set in separate commands
/// still compose into one delivered pair.
#[test]
fn external_child_two_step_set_sees_region_pair() {
    let script = format!(
        "( ulimit -Sn 777; ulimit -Hn 777; {} )",
        child_limits_script()
    );
    let (stdout, stderr, status) = rubash(&script);
    assert_clean(&stderr);
    assert_eq!(status, Some(0));
    assert_eq!(stdout.trim(), "S=777 H=777", "stdout: {stdout}");
}

/// A child spawned from a command substitution inside the region.
#[test]
fn comsub_child_sees_region_pair() {
    let script = format!(
        "x=$(ulimit -n 777; {}); echo \"cap $x\"",
        child_limits_script()
    );
    let (stdout, stderr, status) = rubash(&script);
    assert_clean(&stderr);
    assert_eq!(status, Some(0));
    assert_eq!(stdout.trim(), "cap S=777 H=777", "stdout: {stdout}");
}

/// A child spawned as a pipeline stage's external command.
#[test]
fn pipeline_child_sees_region_pair() {
    let script = format!("{{ ulimit -n 777; {}; }} | cat", child_limits_script());
    let (stdout, stderr, status) = rubash(&script);
    assert_clean(&stderr);
    assert_eq!(status, Some(0));
    assert_eq!(stdout.trim(), "S=777 H=777", "stdout: {stdout}");
}

/// A child spawned by an async list inside the region (the raw-fork
/// spawn route, not the std-Command funnel).
#[test]
fn async_child_sees_region_pair() {
    let script = format!("( ulimit -n 777; {} & wait )", child_limits_script());
    let (stdout, stderr, status) = rubash(&script);
    assert_clean(&stderr);
    assert_eq!(status, Some(0));
    assert_eq!(stdout.trim(), "S=777 H=777", "stdout: {stdout}");
}

/// A process-substitution child born inside the region.
#[test]
fn procsub_child_sees_region_pair() {
    let script = format!("( ulimit -n 777; cat <({}) )", child_limits_script());
    let (stdout, stderr, status) = rubash(&script);
    assert_clean(&stderr);
    assert_eq!(status, Some(0));
    assert_eq!(stdout.trim(), "S=777 H=777", "stdout: {stdout}");
}

/// A coproc child born inside the region.
#[test]
fn coproc_child_sees_region_pair() {
    let script = format!(
        "( ulimit -n 777; coproc {}; cat <&\"${{COPROC[0]}}\" )",
        child_limits_script()
    );
    let (stdout, stderr, status) = rubash(&script);
    assert_clean(&stderr);
    assert_eq!(status, Some(0));
    assert_eq!(stdout.trim(), "S=777 H=777", "stdout: {stdout}");
}

/// The exec builtin's simulated image (a region cannot replace the
/// flat process) is born with the region's pair.
#[test]
fn exec_child_sees_region_pair() {
    let script = format!("( ulimit -n 777; exec {} )", child_limits_script());
    let (stdout, stderr, status) = rubash(&script);
    assert_clean(&stderr);
    assert_eq!(status, Some(0));
    assert_eq!(stdout.trim(), "S=777 H=777", "stdout: {stdout}");
}

/// Control: a child spawned AFTER the region exits sees the ambient
/// pair — the arm applies the region's cells, not a global change.
#[test]
fn child_after_region_exit_sees_ambient_pair() {
    let script = format!(
        "echo \"pre S=$(ulimit -Sn) H=$(ulimit -Hn)\"; ( ulimit -n 777 ); {}",
        child_limits_script()
    );
    let (stdout, stderr, status) = rubash(&script);
    assert_clean(&stderr);
    assert_eq!(status, Some(0));
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "stdout: {stdout}");
    let pre = lines[0].strip_prefix("pre ").unwrap();
    assert_eq!(
        lines[1], pre,
        "post-region child: {} vs ambient {pre}",
        lines[1]
    );
}

/// Ground truth at the kernel boundary (Linux /proc): the child's
/// own /proc/self/limits shows the delivered pair, independent of
/// any shell read-back path.
#[cfg(target_os = "linux")]
#[test]
fn external_child_proc_limits_show_region_pair() {
    let (stdout, stderr, status) =
        rubash("( ulimit -n 777; grep 'Max open files' /proc/self/limits )");
    assert_clean(&stderr);
    assert_eq!(status, Some(0));
    let fields: Vec<&str> = stdout.split_whitespace().collect();
    assert_eq!(fields.len(), 6, "limits line: {stdout}");
    assert_eq!(fields[3], "777", "soft: {stdout}");
    assert_eq!(fields[4], "777", "hard: {stdout}");
}
