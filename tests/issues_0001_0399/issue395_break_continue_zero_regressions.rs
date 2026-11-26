//! Issue rubash#395 regression: `break 0` / `continue 0` used to succeed
//! (rc 0) and plain-break the loop; GNU turns them into a usage failure
//! that still unwinds the loop with status 1.
//!
//! GNU spec:
//! - builtins/break.def:75-83 break_builtin: `newbreak <= 0` ->
//!   sh_erange(arg, "loop count") + `breaking = loop_level` + return
//!   EXECUTION_FAILURE. The loop STILL unwinds.
//! - builtins/break.def:120-128 continue_builtin: same, and it arms
//!   `breaking` (not `continuing`) — `continue 0` ENDS the loop.
//! - execute_cmd.c:635 execute_command_internal: once `breaking` is set,
//!   every later command in the body list early-returns WITHOUT executing;
//!   the loop driver (execute_cmd.c:3840) consumes the flag and breaks.
//! - execute_while_or_until returns body_status (execute_cmd.c:3795), so
//!   the broken loop's status is the break builtin's 1.
//! - break.def:69-70 check_loop_level: outside any loop (function frames
//!   reset loop_level, execute_cmd.c:5358) the builtin short-circuits to
//!   EXECUTION_SUCCESS BEFORE get_numeric_arg ever parses the argument.
//! - builtins/common.c:488-515 get_numeric_arg(list, interactive ? 2 : 1):
//!   a non-numeric count is sh_neednumarg + set_exit_status(EX_BADUSAGE)
//!   + jump_to_top_level(EXITPROG) in a non-interactive script: the whole
//!   shell exits 2.
//!
//! Verified byte-for-byte against WSL GNU Bash 5.3.0 script-file probes
//! (target/issue395/{matrix,fatal,fatal_fn}.sh — stdout, stderr and rc all
//! identical on M1-M14).

#![cfg(windows)]

use std::io::Write;
use std::process::{Command, Stdio};

struct RunOutcome {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_rubash_script(script: &str) -> RunOutcome {
    let dir = std::env::temp_dir().join(format!("rubash-issue395-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue395 scratch dir");
    let script_path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&script_path).expect("write probe script");
    // LF line endings: probes run identically under WSL GNU Bash for
    // baseline comparison.
    file.write_all(script.as_bytes())
        .expect("write probe script");
    drop(file);

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash probe");
    let _ = std::fs::remove_dir_all(&dir);
    RunOutcome {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// GNU matrix M1: `while true; do break 0; done` -> diagnostic + rc 1.
#[test]
fn break_zero_reports_failure_and_ends_loop() {
    let outcome = run_rubash_script("while true; do break 0; done\necho rc=$?\n");
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, "rc=1\n");
    assert!(
        outcome.stderr.contains("break: 0: loop count out of range"),
        "stderr: {}",
        outcome.stderr
    );
}

/// GNU matrix M2/M11: `continue 0` in while and until loops -> rc 1 (the
/// continue.def out-of-range arm arms `breaking`, ending the loop).
#[test]
fn continue_zero_reports_failure_and_ends_loop() {
    let outcome = run_rubash_script(
        "while true; do continue 0; done\necho rc=$?\n\
         n=0\nuntil false; do n=$((n+1)); continue 0; done\necho rc2=$? n=$n\n",
    );
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, "rc=1\nrc2=1 n=1\n");
    assert!(
        outcome
            .stderr
            .contains("continue: 0: loop count out of range"),
        "stderr: {}",
        outcome.stderr
    );
}

/// GNU matrix M3/M4 (execute_cmd.c:635): once `breaking` is armed, the
/// REST of the body list never executes — the marker file is not created.
#[test]
fn break_zero_skips_rest_of_body_list() {
    let outcome = run_rubash_script(
        "while true; do break 0; echo AFTER:$? >> marker.out; break; done\n\
         echo rc=$? marker=$(cat marker.out 2>/dev/null || echo NONE)\n",
    );
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, "rc=1 marker=NONE\n");
}

/// GNU matrix M5: `break -1` is the same out-of-range class.
#[test]
fn break_negative_one_reports_out_of_range() {
    let outcome = run_rubash_script("while true; do break -1; done\necho rc=$?\n");
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, "rc=1\n");
    assert!(
        outcome
            .stderr
            .contains("break: -1: loop count out of range"),
        "stderr: {}",
        outcome.stderr
    );
}

/// GNU matrix M8: a nested `break 0` arms `breaking = loop_level` (2) —
/// the inner loop consumes one level, the surviving flag short-circuits
/// the OUTER body (`OUTER-SEEN` skipped), outer breaks, rc 1.
#[test]
fn break_zero_in_nested_loop_short_circuits_outer_body() {
    let outcome = run_rubash_script(
        "while true; do while true; do break 0; done; echo OUTER-SEEN; break; done\necho rc=$?\n",
    );
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, "rc=1\n");
    assert!(
        !outcome.stdout.contains("OUTER-SEEN"),
        "outer body tail must be skipped"
    );
}

/// GNU matrix M10 + fatal_fn probe (break.def:69-70, execute_cmd.c:5358):
/// inside a function the loop context is gone, so `break 0` /
/// `continue 1x` short-circuit to "only meaningful" + SUCCESS before the
/// argument is even parsed — no fatal exit, function and loop continue.
#[test]
fn break_zero_inside_function_is_loop_local_noop() {
    let outcome = run_rubash_script(
        "f() { break 0; echo FUNC-AFTER; }\n\
         while true; do f; echo AFTER-FUNC:$?; break; done\necho rc=$?\n",
    );
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, "FUNC-AFTER\nAFTER-FUNC:0\nrc=0\n");
    assert!(
        outcome
            .stderr
            .contains("break: only meaningful in a `for', `while', or `until' loop"),
        "stderr: {}",
        outcome.stderr
    );
}

/// GNU fatal probe (common.c:488-515, fatal=1 non-interactive): a
/// non-numeric count is sh_neednumarg + EXITPROG — the whole script dies
/// with rc 2 at the first `break 2x`.
#[test]
fn break_non_numeric_count_exits_script_with_two() {
    let outcome = run_rubash_script("echo before\nwhile true; do break 2x; done\necho NEVER\n");
    assert_eq!(outcome.code, Some(2));
    assert_eq!(outcome.stdout, "before\n");
    assert!(
        outcome
            .stderr
            .contains("break: 2x: numeric argument required"),
        "stderr: {}",
        outcome.stderr
    );
}

/// Guard: a normal in-range `break 1`/`break 2` still unwinds with rc 0
/// (GNU matrix M7/M13).
#[test]
fn normal_break_still_succeeds() {
    let outcome = run_rubash_script(
        "while true; do break 1; done\necho rc=$?\n\
         while true; do while true; do break 2; done; echo INNER-NEVER; done\necho rc2=$?\n",
    );
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, "rc=0\nrc2=0\n");
    assert!(
        !outcome.stdout.contains("INNER-NEVER"),
        "in-range break 2 must skip the outer tail"
    );
    assert!(outcome.stderr.is_empty(), "stderr: {}", outcome.stderr);
}
