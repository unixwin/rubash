//! rubash#4059 — `printf '%(%s)T' -2` must report the true unshifted shell
//! start time even when `SECONDS` is imported from the initial environment.
//!
//! GNU shell.c:470 sets `shell_start_time = shellstart.tv_sec` at startup, and
//! variables.c:1349-1361 `init_seconds_var` seeds `seconds_value_assigned`
//! from an inherited `SECONDS` (falling back to 0 when not a valid number)
//! without modifying `shell_start_time`. `builtins/printf.def:626-627` then
//! reads `shell_start_time` directly for `-2`.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after unix epoch")
        .as_secs() as i64
}

fn rubash_with_seconds(imported: Option<&str>, script: &str) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    if let Some(val) = imported {
        cmd.env("SECONDS", val);
    } else {
        cmd.env_remove("SECONDS");
    }
    let output = cmd.arg("-c").arg(script).output().expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn printf_t2_is_unshifted_when_seconds_imported_in_env() {
    let before = now_epoch();
    let (stdout, stderr, code) = rubash_with_seconds(
        Some("10"),
        "printf '%(%s)T\\n%(%s)T\\n%s\\n%s\\n' -2 -1 \"$SECONDS\" \"$EPOCHSECONDS\"",
    );
    let after = now_epoch();
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");

    let nums: Vec<i64> = stdout
        .lines()
        .map(|line| line.trim().parse::<i64>().expect("numeric line"))
        .collect();
    assert_eq!(nums.len(), 4, "stdout: {stdout}");
    let (t2, t1, sec, epoch) = (nums[0], nums[1], nums[2], nums[3]);

    assert!(
        (before..=after).contains(&t2),
        "printf %T -2 ({t2}) must be unshifted shell start time in [{before}, {after}]"
    );
    assert!(
        (before..=after).contains(&t1),
        "now ({t1}) in [{before}, {after}]"
    );
    assert!(
        (10..=10 + (after - before) + 1).contains(&sec),
        "SECONDS ({sec}) must start at imported 10"
    );
    assert!(
        (before..=after).contains(&epoch),
        "EPOCHSECONDS ({epoch}) must be unshifted in [{before}, {after}]"
    );
}

#[test]
fn imported_seconds_edge_values_and_reassignment() {
    let before = now_epoch();
    let (stdout_neg, stderr_neg, code_neg) =
        rubash_with_seconds(Some("-5"), "printf '%(%s)T %s\\n' -2 \"$SECONDS\"");
    let after_neg = now_epoch();
    assert_eq!(code_neg, Some(0), "stderr: {stderr_neg}");
    let parts: Vec<i64> = stdout_neg
        .split_whitespace()
        .map(|s| s.parse().expect("numeric"))
        .collect();
    assert_eq!(parts.len(), 2, "stdout: {stdout_neg}");
    assert!((before..=after_neg).contains(&parts[0]));
    assert!((-5..=-3).contains(&parts[1]));

    let before_bad = now_epoch();
    let (stdout_bad, stderr_bad, code_bad) =
        rubash_with_seconds(Some("abc"), "printf '%(%s)T %s\\n' -2 \"$SECONDS\"");
    let after_bad = now_epoch();
    assert_eq!(code_bad, Some(0), "stderr: {stderr_bad}");
    let parts_bad: Vec<i64> = stdout_bad
        .split_whitespace()
        .map(|s| s.parse().expect("numeric"))
        .collect();
    assert_eq!(parts_bad.len(), 2, "stdout: {stdout_bad}");
    assert!((before_bad..=after_bad).contains(&parts_bad[0]));
    assert!((0..=2).contains(&parts_bad[1]));

    let before_assign = now_epoch();
    let (stdout_assign, stderr_assign, code_assign) = rubash_with_seconds(
        Some("10"),
        "SECONDS=42; printf '%(%s)T %s\\n' -2 \"$SECONDS\"",
    );
    let after_assign = now_epoch();
    assert_eq!(code_assign, Some(0), "stderr: {stderr_assign}");
    let parts_assign: Vec<i64> = stdout_assign
        .split_whitespace()
        .map(|s| s.parse().expect("numeric"))
        .collect();
    assert_eq!(parts_assign.len(), 2, "stdout: {stdout_assign}");
    assert!((before_assign..=after_assign).contains(&parts_assign[0]));
    assert!((42..=44).contains(&parts_assign[1]));
}

#[test]
fn declare_p_does_not_leak_internal_seconds_or_start_epoch_keys() {
    let (stdout, stderr, code) = rubash_with_seconds(Some("10"), "declare -p");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        !stdout.contains("__RUBASH_SECONDS_OFFSET")
            && !stdout.contains("__RUBASH_SHELL_START_EPOCH"),
        "declare -p leaked internal SECONDS/epoch key:\n{stdout}"
    );
}
