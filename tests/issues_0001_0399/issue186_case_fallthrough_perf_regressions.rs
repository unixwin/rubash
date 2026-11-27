//! rubash#186 — case fallthrough performance collapse (sentinel).
//!
//! GNU execute_cmd.c evaluates a `case` by scanning the pattern list
//! once per input; the filed bug re-scanned/re-parsed per clause,
//! making long fallthrough chains quadratic. This is a SENTINEL in
//! the shape of tests/issue1119_1264_perf_scale_regressions.rs: it
//! checks the SCALING SHAPE (doubling the chain length must not
//! multiply the time by an order of magnitude) with a generous
//! absolute ceiling, never an absolute-ms equality. Correctness of
//! the fallthrough result is asserted exactly.
//!
//! The shape bound follows the repo's own perf-sentinel precedent;
//! absolute timings vary by machine and are not part of the pin.

use std::process::Command;
use std::time::{Duration, Instant};

fn run_chain(clauses: usize) -> Duration {
    let mut script = String::from("case zzz in\n");
    for i in 0..clauses {
        script.push_str(&format!("pat{i}) echo hit{i};;\n"));
    }
    script.push_str("*) echo fell-through;;\nesac\n");
    let start = Instant::now();
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(&script)
        .output()
        .expect("run rubash");
    let elapsed = start.elapsed();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(stdout, "fell-through\n");
    assert_eq!(output.status.code(), Some(0));
    elapsed
}

#[test]
fn case_fallthrough_scales_near_linearly() {
    // Warm-up so process spawn/JIT-ish costs do not dominate.
    let _ = run_chain(50);
    let small = run_chain(200);
    let large = run_chain(800);
    // 4x the clauses must cost far less than 16x (quadratic) — allow
    // a generous 10x ceiling plus the small run's noise floor.
    let ceiling = small.saturating_mul(10) + Duration::from_millis(500);
    assert!(
        large < ceiling,
        "case fallthrough scaling looks super-linear: 200 clauses {small:?}, 800 clauses {large:?}"
    );
    assert!(
        large < Duration::from_secs(20),
        "800-clause fallthrough must stay far under the filed collapse: {large:?}"
    );
}
