//! rubash#2932 — `times` must print the process's real rusage
//! values, not the stub's literal zeros.
//!
//! GNU anchor: builtins/times.def:65-88 — `times_builtin` calls
//! `getrusage(RUSAGE_SELF)` and prints its user/system times as the
//! first line, then `getrusage(RUSAGE_CHILDREN)` for the second,
//! each value through `print_timeval` (lib/sh/timeval.c:158-171,
//! `"%ldm%d%c%03lds"`: truncated milliseconds, exactly three
//! fraction digits).
//!
//! The pins burn a bounded amount of CPU — in-shell arithmetic for
//! the SELF line, a waited external child for the CHILDREN line —
//! and require the corresponding line's user+sys total to be
//! nonzero, which the stub could never produce.

use std::process::Command;

fn run_script(script: &str) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

/// `{mins}m{secs}.{millis:03}s` -> total milliseconds.
fn timeval_millis(field: &str) -> u64 {
    let (mins, rest) = field.split_once('m').expect("minutes separator");
    let (secs, frac) = rest.split_once('.').expect("decimal point");
    let frac = frac.strip_suffix('s').expect("seconds suffix");
    assert_eq!(frac.len(), 3, "field: {field}");
    mins.parse::<u64>().expect("minutes") * 60_000
        + secs.parse::<u64>().expect("seconds") * 1_000
        + frac.parse::<u64>().expect("millis")
}

fn line_total_millis(line: &str) -> u64 {
    line.split(' ').map(timeval_millis).sum()
}

// 20k arithmetic iterations ≈ seconds of CPU in a debug build and
// tens of milliseconds in a release build — either way far above
// print_timeval's 1ms truncation, which is all the >0 pins need.
const BURN: &str = "i=0; while (( i < 20000 )); do (( i += 1 )); done";

#[test]
fn times_reports_shell_cpu_after_arithmetic_burn() {
    let (ok, stdout) = run_script(&format!("{BURN}; times"));
    assert!(ok, "script failed, stdout: {stdout}");
    let self_line = stdout.lines().next().expect("first times line");
    assert!(
        line_total_millis(self_line) > 0,
        "SELF user+sys must be nonzero after an in-shell CPU burn, got: {self_line}"
    );
}

#[test]
fn times_reports_children_cpu_after_external_burn() {
    let bin = env!("CARGO_BIN_EXE_rubash");
    let script = format!("\"{bin}\" -c '{BURN}'; times");
    let (ok, stdout) = run_script(&script);
    assert!(ok, "script failed, stdout: {stdout}");
    let children_line = stdout.lines().nth(1).expect("second times line");
    assert!(
        line_total_millis(children_line) > 0,
        "CHILDREN user+sys must be nonzero after a waited CPU-burning child, got: {children_line}"
    );
}

#[test]
fn times_shape_and_status() {
    let (ok, stdout) = run_script("times");
    assert!(ok);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "stdout: {stdout}");
    for line in lines {
        // Parses as two print_timeval fields (panics otherwise).
        let _ = line_total_millis(line);
    }

    // Operands are ignored (times.def runs no_options; operands fall
    // through) and the shape is unchanged.
    let (ok, stdout) = run_script("times ignored");
    assert!(ok);
    assert_eq!(stdout.lines().count(), 2, "stdout: {stdout}");
}
