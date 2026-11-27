//! rubash#3230 — bats' gather phase is pathologically slow on master:
//! every declare-family dispatch (including each `local` inside a
//! function) eagerly rendered the ENTIRE function table's listing
//! text, although only a bare `declare` / bare `set` listing ever
//! prints it. bats loads ~150 library functions and calls `local`
//! pervasively, so the tax is O(functions x body size) per call
//! (measured on X299, release: 2,000 local-carrying calls under 150
//! twelve-line functions took ~33 s before the fix, ~1 s after; bats'
//! own per-test accounting went 221 ms -> 37 ms, the pre-regression
//! magnitude).
//!
//! Root cause: the #3487 work (PR #3530) threaded pre-rendered
//! FunctionSetViews through the set/declare funnels and the executor
//! materialized them at every dispatch. GNU renders function
//! definitions only inside print_all_shell_variables'
//! print_func_list arm (builtins/set.def:506-527;
//! variables.c:1079-1090), reached solely from the no-operand
//! listing. The fix makes the views a lazy provider
//! (FunctionSetViewProvider) that the funnels realize only in that
//! arm, so operand-carrying dispatches never render.
//!
//! Pin shape (per the perf-pin convention in
//! tests/issue1119_1264_perf_scale_regressions.rs): FUNCTION plus an
//! order-of-magnitude sentinel. The sentinel is a scaling ratio —
//! the same local-loop must cost about the same with 20 functions
//! defined as with 160; under the regression it grows ~linearly with
//! the table (~8x at these sizes). No absolute-millisecond
//! expectations; a generous absolute backstop catches flat blowups.

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

/// Define `nfun` functions with ten-command bodies, then run a
/// 4,000-iteration loop whose worker performs three `local`
/// assignments per call. Prints the accumulated sum so the test can
/// assert the loop's FUNCTION, not just its cost.
fn local_loop_script(nfun: usize) -> String {
    format!(
        "i=1; while [ $i -le {nfun} ]; do \
           eval \"big$i() {{ echo L1; echo L2; echo L3; echo L4; echo L5; \
           echo L6; echo L7; echo L8; echo L9; echo L10; }}\"; \
           i=$((i+1)); done; \
         s=0; \
         w() {{ local a=1; local b=2; local c=$((a+b)); s=$((s+c)); }}; \
         i=0; while [ $i -lt 4000 ]; do w; i=$((i+1)); done; \
         echo \"s=$s\""
    )
}

#[test]
fn local_loop_cost_does_not_scale_with_function_table_size() {
    let run = |nfun: usize| -> Duration {
        // Min of two runs: process-startup noise must not decide a
        // complexity-class bound.
        let mut best: Option<Duration> = None;
        for _ in 0..2 {
            let (stdout, _stderr, code, elapsed) = rubash_timed(&local_loop_script(nfun));
            assert_eq!(code, Some(0));
            assert_eq!(
                stdout.trim(),
                "s=12000",
                "local loop function (nfun={nfun})"
            );
            best = Some(best.map_or(elapsed, |b: Duration| b.min(elapsed)));
        }
        best.expect("at least one run")
    };
    let t_small = run(20);
    let t_large = run(160);
    assert!(
        t_large < Duration::from_secs(120),
        "absolute backstop: 160-function local loop took {t_large:?}"
    );
    assert!(
        t_large < t_small * 3,
        "local-loop cost scales with the function table \
         (20 functions: {t_small:?}, 160 functions: {t_large:?}); \
         declare-family dispatch must not render function listings \
         for operand-carrying calls (rubash#3230)"
    );
}

fn run_script(script: &str) -> (String, Option<i32>) {
    // Script-file invocation (the tests/issue3487 harness shape): a
    // `-c` child would carry BASH_EXECUTION_STRING — the script text
    // itself, marker lines included — inside its own listings, which
    // makes arm-slicing by marker text meaningless.
    let dir =
        std::env::temp_dir().join(format!("rubash-issue3230-listings-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .stdin(std::process::Stdio::null())
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("TERM", "xterm")
        .env("HOME", &dir)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.code(),
    )
}

#[test]
fn bare_listings_still_carry_functions_in_a_function_heavy_shell() {
    // The lazy provider must still fire on the two listing arms (and
    // only there) when the table is heavy: bare `declare` and bare
    // `set` print the definitions; an operand form does not.
    let script = "i=1; while [ $i -le 30 ]; do \
                    eval \"big$i() { echo body$i; }\"; i=$((i+1)); done\n\
                  marker_var=present\n\
                  echo '--- declare'\ndeclare\n\
                  echo '--- set'\nset\n\
                  echo '--- declare-p'\ndeclare -p marker_var\n";
    let (stdout, code) = run_script(script);
    assert_eq!(code, Some(0));
    let declare_arm = stdout.split("--- set").next().expect("declare arm");
    assert!(
        declare_arm.contains("big30 ()"),
        "bare declare must print function definitions:\n{stdout}"
    );
    // splitn(2, ..) for the set marker: the set listing itself
    // contains a `_='--- set'` variable line, and the function
    // section prints after all variables, so a plain split + nth(1)
    // would cut the arm before the definitions.
    let set_arm = stdout
        .split("--- declare-p")
        .next()
        .expect("set arm")
        .splitn(2, "--- set")
        .nth(1)
        .expect("set arm body");
    assert!(
        set_arm.contains("big30 ()"),
        "bare set must print function definitions:\n{stdout}"
    );
    let p_arm = stdout
        .split("--- declare-p")
        .nth(1)
        .expect("declare -p arm");
    assert!(
        !p_arm.contains("big30 ()"),
        "declare -p must not print function definitions:\n{stdout}"
    );
}
