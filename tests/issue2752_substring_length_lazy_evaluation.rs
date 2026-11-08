//! rubash#2752 (deep-audit B6-R15) — substring expansion must evaluate
//! the length lazily: GNU evaluates it only when the offset is in range
//! for the value; rubash evaluates offset and length eagerly in the
//! routing parser, so an out-of-range offset over a garbage or
//! side-effecting length errors (or mutates state) where GNU silently
//! expands to empty.
//!
//! GNU anchors: `parameter_brace_substring` (subst.c:9050) returns NULL
//! before any evaluation for an unset value (that face is rubash#3024,
//! already fixed — B7 below is its regression lock). For a set value,
//! `verify_substring_values` (subst.c:8395) evaluates the offset first,
//! computes the value length, and returns -1 — empty result, no
//! diagnostic, length never evaluated — when the offset is out of range
//! (`*e1p > len || *e1p < 0`); the length text is only expanded and
//! evaluated after that check. For a non-empty value with an in-range
//! offset both shells error identically (B6 control).
//!
//! Expectations byte-measured against GNU bash 5.3.20. Harness mirrors
//! issue3024: `rubash -c` via `CARGO_BIN_EXE_rubash`, `LC_ALL=C.UTF-8`.

use std::process::{Command, Stdio};

struct Outcome {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    code: i32,
}

fn run(script: &str) -> Outcome {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    Outcome {
        stdout: output.stdout,
        stderr: output.stderr,
        code: output.status.code().unwrap_or(-1),
    }
}

/// Out-of-range offset: the whole expansion is empty, rc 0, no stderr,
/// and stdout is exactly the expected bytes (affixes + any follow-up).
fn expect_empty_ok(script: &str, expected_stdout: &[u8], ctx: &str) {
    let out = run(script);
    assert_eq!(
        out.code, 0,
        "{ctx}: rc must be 0; stdout: {:?} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        out.stdout, expected_stdout,
        "{ctx}: stdout mismatch; got {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        out.stderr.is_empty(),
        "{ctx}: stderr must be empty (length never evaluated); got {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn set_null_out_of_range_offset_garbage_length_is_empty() {
    // The issue's own repro, chained: the following command still runs
    // and observes rc 0 for the expansion line.
    expect_empty_ok(
        "v=; echo \"X${v:1:2:3}Y\"; echo \"after rc=$?\"",
        b"XY\nafter rc=0\n",
        "B14 set-but-null v, offset 1 > len 0, garbage length 2:3",
    );
}

#[test]
fn short_value_out_of_range_offset_garbage_length_is_empty() {
    expect_empty_ok(
        "v=ab; echo \"X${v:5:2:3}Y\"",
        b"XY\n",
        "B2 v=ab, offset 5 out of range, garbage length",
    );
}

#[test]
fn out_of_range_offset_length_side_effect_never_runs() {
    // Chained state assertion: the length's `$((x+=1))` must not run, so
    // x is still unset on the next line.
    expect_empty_ok(
        "v=ab; echo \"X${v:5:$((x+=1))}Y\"; echo \"x=${x-unset}\"",
        b"XY\nx=unset\n",
        "B3 out-of-range offset, side-effecting length",
    );
}

#[test]
fn out_of_range_offset_bad_name_length_is_empty() {
    expect_empty_ok(
        "v=ab; echo \"X${v:5:zz}Y\"",
        b"XY\n",
        "B4 out-of-range offset, unresolvable length name",
    );
}

#[test]
fn array_out_of_range_offset_garbage_length_is_empty() {
    expect_empty_ok(
        "arr2=(x y); echo \"X${arr2[@]:9:1:9}Y\"",
        b"XY\n",
        "B5 array offset beyond max index, garbage length",
    );
}

#[test]
fn in_range_offset_garbage_length_still_errors_control() {
    // Control: offset 1 is in range for "abcdef", so GNU *does* evaluate
    // the length "2:3" and errors — identically to the eager behavior.
    let out = run("v=abcdef; echo \"X${v:1:2:3}Y\"");
    assert_eq!(
        out.code, 1,
        "B6: rc must be 1; stdout: {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("v: 2:3: arithmetic syntax error in expression (error token is \":3\")"),
        "B6: stderr must carry the length evaluation error; got {stderr}"
    );
}

#[test]
fn unset_value_side_effect_offset_never_runs_regression_lock() {
    // rubash#3024 face (GNU subst.c:9058): an unset value skips even the
    // offset evaluation. Already fixed on master; this cell locks it so
    // the lazy-length rework cannot regress it.
    expect_empty_ok(
        "unset v; echo \"X${v:$((x+=1)):2:3}Y\"; echo \"x=${x-unset}\"",
        b"XY\nx=unset\n",
        "B7 unset v, side-effecting offset (regression lock)",
    );
}

#[test]
fn negative_out_of_range_offset_garbage_length_is_empty() {
    expect_empty_ok(
        "v=ab; echo \"X${v: -5:2:3}Y\"",
        b"XY\n",
        "B8 negative offset -5 + len 2 still < 0, garbage length",
    );
}

#[test]
fn positional_out_of_range_offset_garbage_length_is_empty() {
    expect_empty_ok(
        "set -- a b; echo \"X${@:5:2:3}Y\"",
        b"XY\n",
        "B9 positional offset 5 beyond $# + 1, garbage length",
    );
}

#[test]
fn in_range_offset_length_side_effect_still_runs_control() {
    // Control chain: in-range offset, so the length IS evaluated — the
    // increment happens and the slice uses its value.
    expect_empty_ok(
        "v=abcdef; echo \"X${v:1:$((x+=1))}Y\"; echo \"x=${x-0}\"",
        b"XbY\nx=1\n",
        "B10 in-range offset, side-effecting length (control)",
    );
}

#[test]
fn assoc_array_out_of_range_offset_garbage_length_is_empty() {
    expect_empty_ok(
        "declare -A m=([a]=1 [b]=2); echo \"X${m[@]:9:1:9}Y\"",
        b"XY\n",
        "B11 assoc array offset beyond element count, garbage length",
    );
}

#[test]
fn empty_array_out_of_range_offset_garbage_length_is_empty() {
    expect_empty_ok(
        "arr2=(); echo \"X${arr2[@]:1:2:3}Y\"",
        b"XY\n",
        "B12 set-but-empty array, offset 1, garbage length",
    );
}

#[test]
fn out_of_range_offset_length_comsub_never_runs() {
    // The length's command substitution must not execute at all: no
    // MARKER on stderr, empty expansion, rc 0.
    let out = run("v=ab; echo \"X${v:5:$(echo MARKER >&2)}Y\"");
    assert_eq!(out.code, 0, "B13: rc must be 0");
    assert_eq!(
        out.stdout,
        b"XY\n",
        "B13: stdout must be just the affixes; got {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        !String::from_utf8_lossy(&out.stderr).contains("MARKER"),
        "B13: the length comsub must not execute; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
