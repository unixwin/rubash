//! rubash#2685 — nested arithmetic expansion failed at depth 52
//! (`arithmetic syntax error: operand expected` naming the innermost
//! unexpanded `$((...))`), where GNU bash 5.3.20 evaluates depth 500.
//!
//! Root cause (diagnosed 2026-11-08): the embedded-parameter walker's
//! thread-local recursion guard `MAX_EXPAND_DEPTH = 50`
//! (src/executor/embedded_parameters.rs) returns the word UNCHANGED
//! once the depth is reached. Each nested `$((...))` level re-enters
//! the walker once (eval_arithmetic_expansion_value ->
//! expand_arithmetic_expression_mut ->
//! expand_arithmetic_special_parameters ->
//! expand_embedded_parameters), so at 52 levels the innermost span
//! survives as literal text and the arithmetic parser fails it as an
//! operand. GNU has no such fixed cap: subst.c:10915/10926 recurses
//! expand_arith_string/evalexp on the C stack.
//!
//! STATUS: fail-first verified on master 8f4daa2d (Simular box,
//! 2026-11-10): depth_52 / depth_100 / depth_500 /
//! pure_nesting_depth_60 red, depth_51 control green. The fix
//! (MAX_EXPAND_DEPTH 50 -> 1000, measured-memory rationale) rides in
//! the same PR as this pin. First written against base 88047f50 (A1
//! lane); the guard file was then a live file of PR #3113, since
//! merged (c52d3164).
//!
//! Harness: `rubash -c` via `CARGO_BIN_EXE_rubash`. `LC_ALL=C.UTF-8`
//! is set on the child explicitly so the pin is hermetic.

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

/// `$((1+ $((1+ ... 1)) ))` with `levels` nested expansions.
fn nested_sum(levels: usize) -> String {
    let mut text = "1".to_string();
    for _ in 0..levels {
        text = format!("$((1+{text}))");
    }
    text
}

fn expect_nested_sum(levels: usize) {
    let script = format!("echo {}", nested_sum(levels));
    let out = run(&script);
    let expected = format!("{}\n", levels + 1).into_bytes();
    assert_eq!(
        out.stdout,
        expected,
        "depth {levels}: stdout mismatch; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        out.code,
        0,
        "depth {levels}: rc must be 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stderr.is_empty(),
        "depth {levels}: stderr must be empty; got {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn depth_51_still_works() {
    expect_nested_sum(51);
}

#[test]
fn depth_52_evaluates() {
    expect_nested_sum(52);
}

#[test]
fn depth_100_evaluates() {
    expect_nested_sum(100);
}

#[test]
fn depth_500_matches_gnu() {
    expect_nested_sum(500);
}

#[test]
fn pure_nesting_depth_60_evaluates() {
    let mut text = "7".to_string();
    for _ in 0..60 {
        text = format!("$(( {text} ))");
    }
    let out = run(&format!("echo {text}"));
    assert_eq!(out.stdout, b"7\n", "pure nesting depth 60");
    assert_eq!(out.code, 0, "pure nesting depth 60 rc");
    assert!(
        out.stderr.is_empty(),
        "pure nesting depth 60: stderr must be empty; got {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
