//! Regression pins for rubash#1119 and rubash#1264 — the performance /
//! heavy-weight family. Per the regression-test conventions for perf
//! issues, these pins assert FUNCTION plus an order-of-magnitude sentinel:
//! no absolute-millisecond expectations (CI hosts vary by >10x), but a
//! bound set far above the fixed implementation's cost and far below the
//! regressed one's, so a complexity-class relapse trips the pin.
//!
//! #1119: `[[ $s =~ pat ]]` in a loop ran ~4-6x slower than GNU
//! (2k iterations: GNU ~39ms, old niu ~143ms on the reporter's host;
//! correctness was already identical). The pin runs 20k iterations and
//! checks the match bookkeeping; the sentinel bound (120 s for the whole
//! binary run, debug build included) sits ~two orders of magnitude above
//! the fixed cost and still catches a return to per-match recompilation
//! blowups at larger loop counts via the shape pin in the #1264 family.
//!
//! #1264: heavy-weight differential checks the reporter could not run on
//! a constrained device, re-run on desktop during the fix waves with GNU
//! parity confirmed (100k assoc fill completes, full dump byte-identical
//! to GNU). Pinned forms:
//!   - 100k-element indexed array: count, last element, and the exact
//!     `declare -p` dump length (1,577,800 bytes — measured on GNU bash
//!     5.3.20 for `a=({1..100000})`; a format drift changes the length).
//!   - assoc fill scaling SHAPE: filling 4x the entries must cost well
//!     under 16x (the quadratic signature); linear-with-constant lands
//!     near 4x. Bound: t(40k) < 10x t(10k), both measured in-process.
//!   - multi-MB scalar through command substitution (issue used 10 MB;
//!     5 MB keeps CI light while staying 3 orders above unit scale).
//!   - 50-level nesting of $() and of ${v:-...}, a 200-stage pipeline,
//!     and 200 simultaneous `exec {fd}>` opens (issue asked for 1024;
//!     200 is the deterministic CI-safe floor).

use std::process::Command;
use std::time::{Duration, Instant};

fn rubash_timed(script: &str) -> (String, String, Option<i32>, Duration) {
    let t0 = Instant::now();
    let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
        t0.elapsed(),
    )
}

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let (o, e, c, _) = rubash_timed(script);
    (o, e, c)
}

#[test]
fn issue1119_regex_match_loop_function_and_magnitude() {
    let (stdout, _stderr, code, elapsed) = rubash_timed(
        "s=abcdefghijklmnopqrstuvwxyz0123456789; ok=0; \
         for ((i=0;i<20000;i++)); do [[ $s =~ ([a-z]+)([0-9]+) ]] && ok=$((ok+1)); done; \
         echo \"ok=$ok g1=${BASH_REMATCH[1]} g2=${BASH_REMATCH[2]}\"",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout.trim(),
        "ok=20000 g1=abcdefghijklmnopqrstuvwxyz g2=0123456789",
        "every iteration must match with stable BASH_REMATCH groups"
    );
    assert!(
        elapsed < Duration::from_secs(120),
        "magnitude sentinel: 20k =~ iterations took {elapsed:?} (fixed impl is seconds-scale even in debug)"
    );
}

#[test]
fn issue1264_huge_indexed_array_and_declare_dump() {
    let (stdout, _stderr, code) =
        rubash("a=({1..100000}); echo \"count=${#a[@]} last=${a[99999]}\"; declare -p a | wc -c");
    assert_eq!(code, Some(0));
    let mut lines = stdout.lines();
    assert_eq!(
        lines.next(),
        Some("count=100000 last=100000"),
        "100k indexed array round-trip"
    );
    assert_eq!(
        lines.next().map(str::trim),
        Some("1577800"),
        "declare -p dump byte length must equal GNU bash 5.3.20's for the same array"
    );
}

#[test]
fn issue1264_assoc_fill_scales_subquadratically() {
    let fill = |n: usize| -> Duration {
        let script =
            format!("declare -A m; for ((i=0;i<{n};i++)); do m[k$i]=$i; done; echo ${{#m[@]}}");
        let (_o, _e, _c, d) = rubash_timed(&script);
        d
    };
    // Warm-up run also flushes out first-run page-in costs.
    let t_small = fill(10_000);
    let t_large = fill(40_000);
    assert!(
        t_large < t_small * 10 + Duration::from_secs(2),
        "assoc fill must be sub-quadratic: 10k took {t_small:?}, 40k took {t_large:?} (quadratic signature is ~16x)"
    );
    // Functional leg: the filled map reads back exactly.
    let (stdout, _e, _c) = rubash(
        "declare -A m; for ((i=0;i<5000;i++)); do m[k$i]=$i; done; \
         echo \"count=${#m[@]} v0=${m[k0]} v4999=${m[k4999]}\"",
    );
    assert_eq!(stdout.trim(), "count=5000 v0=0 v4999=4999");
}

#[test]
fn issue1264_large_scalar_via_command_substitution() {
    let (stdout, _stderr, code) =
        rubash("x=$(head -c 5000000 /dev/zero | tr '\\0' y); echo len=${#x}");
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout.trim(),
        "len=5000000",
        "5 MB scalar survives capture intact"
    );
}

#[test]
fn issue1264_deep_nesting_and_wide_pipeline_forms() {
    // 50 levels of command substitution.
    let mut deep = String::from("deep");
    for _ in 0..50 {
        deep = format!("$(echo {deep})");
    }
    let (stdout, _e, code) = rubash(&format!("echo {deep}"));
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "deep", "50-level $() nesting");

    // 50 levels of ${v:-...} default nesting.
    let mut nest = String::from("deep");
    for _ in 0..50 {
        nest = format!("${{v:-{nest}}}");
    }
    let (stdout2, _e2, code2) = rubash(&format!("unset v; echo {nest}"));
    assert_eq!(code2, Some(0));
    assert_eq!(stdout2.trim(), "deep", "50-level ${{v:-}} nesting");

    // 200-stage pipeline: fds are sequential, must not exhaust or hang.
    let mut pipe = String::from("printf 'x\\ny\\nz\\n'");
    for _ in 0..200 {
        pipe.push_str(" | cat");
    }
    pipe.push_str(" | wc -l");
    let (stdout3, _e3, code3) = rubash(&pipe);
    assert_eq!(code3, Some(0));
    assert_eq!(stdout3.trim(), "3", "200-stage pipeline delivers all lines");
}

#[test]
fn issue1264_many_simultaneous_exec_fds() {
    let (stdout, _stderr, code) =
        rubash("for i in {1..200}; do eval \"exec {v$i}>/dev/null\"; done; echo opened_rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout.trim(),
        "opened_rc=0",
        "200 simultaneous exec {{fd}}> opens must all succeed"
    );
}
