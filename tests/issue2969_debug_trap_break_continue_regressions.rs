//! rubash#2969 — `break`/`continue` inside a DEBUG trap action are
//! GNU's *pending flags*, not an immediate unwind.
//!
//! GNU mechanism (bash 5.3 sources):
//! - builtins/break.def:55,58 — the globals `breaking` / `continuing`;
//!   break_builtin (break.def:88) and continue_builtin (break.def:133)
//!   ONLY assign the flag (`breaking = newbreak`), clamped to
//!   loop_level; check_loop_level (break.def:141-145) prints
//!   "only meaningful in a `for', `while', or `until' loop" and sets
//!   nothing when loop_level == 0.
//! - execute_cmd.c:635-636 execute_command_internal — the entry gate:
//!   `if (breaking || continuing) return (last_command_exit_value);`
//!   A command entered while a flag is pending is skipped whole (no
//!   DEBUG firing for it); a command whose own firing SET the flag was
//!   already entered, so it still runs to completion.
//! - Consumption: execute_cmd.c:3108-3118 (execute_for post-body),
//!   :3252-3290 (arith-for), :3840-3850 (execute_while_or_until
//!   post-body) and :3819-3831 (while/until test-failure drain):
//!   each loop level consumes one count after the body.
//! - The for-head firing (execute_cmd.c:3053) happens INSIDE
//!   execute_for, where loop_level is already 1 (execute_for
//!   increments it at entry), so a trap `break` there is in-loop:
//!   it sets the flag, the body command is skipped by the entry
//!   gate, and the post-body consumption exits the loop — no
//!   diagnostic, no body output.
//!
//! Pre-fix rubash returned Err(Break/Continue) from the builtin, and
//! run_debug_trap propagated it through the firing site: at a
//! for-head firing the loop frame was not yet counted
//! (loop_depth == 0), so the builtin misreported "only meaningful"
//! and the loop ran on; at a body-command firing the unwind aborted
//! the pending command itself (the command was swallowed).
//!
//! Every expectation below was measured against GNU bash 5.3.20
//! (r81 hunt morphology, two identical passes). All cases run as
//! `rubash -c <script>` with LC_ALL=C; exit status is 0 throughout.
//! stderr is asserted by counting the "only meaningful" diagnostic
//! lines (the shell-name/line prolog differs between the oracle and
//! rubash by invocation, the diagnostic text and count do not).

use std::process::{Command, Stdio};
use std::time::Duration;

mod common;
use common::wait_bounded;

fn run_case(script: &str) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(["-c", script])
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    let out = wait_bounded(&mut child, Duration::from_secs(20));
    assert!(!out.timed_out, "case timed out: {script}");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.code,
    )
}

/// Assert the exact stdout and the number of "only meaningful"
/// diagnostics on stderr for one `-c` case.
fn assert_case(script: &str, want_stdout: &str, want_meaningful: usize) {
    let (stdout, stderr, code) = run_case(script);
    assert_eq!(
        code,
        Some(0),
        "exit status for {script:?} (stderr: {stderr:?})"
    );
    assert_eq!(stdout, want_stdout, "stdout for {script:?}");
    let got = stderr.matches("only meaningful").count();
    assert_eq!(
        got, want_meaningful,
        "stderr diagnostic count for {script:?} (stderr: {stderr:?})"
    );
}

// ---- The issue's regression seed (4 cases) ----

#[test]
fn seed_break_at_for_head_firing_exits_loop_silently() {
    // Face A: break on the 1st firing (the for head) — the body never
    // runs, no diagnostic.
    assert_case(
        "c=0; trap 'c=$((c+1)); [ $c -eq 1 ] && break' DEBUG; for i in 1 2 3; do echo \"in $i\"; done; echo after",
        "after\n",
        0,
    );
}

#[test]
fn seed_continue_at_body_firing_does_not_swallow_command() {
    // Face B: continue on the 2nd firing (the body echo's own firing)
    // — the echo was already entered, so it runs; every line prints.
    assert_case(
        "c=0; trap 'c=$((c+1)); [ $c -eq 2 ] && continue' DEBUG; for i in 1 2 3; do echo \"in $i\"; done; echo after",
        "in 1\nin 2\nin 3\nafter\n",
        0,
    );
}

#[test]
fn seed_unconditional_break_reports_once() {
    // bc11: an unconditional trap `break` reports exactly once (at
    // the first post-loop firing, where loop_level is 0 again).
    assert_case(
        "trap 'break' DEBUG; for i in 1 2 3; do echo \"in $i\"; done; echo after",
        "after\n",
        1,
    );
}

#[test]
fn seed_firing_count_control() {
    // Pure MARK trap (no loop control): two firings per iteration —
    // the for head and the body command.
    assert_case(
        "trap 'echo MARK' DEBUG; for i in 1 2; do echo \"in $i\"; done",
        "MARK\nMARK\nin 1\nMARK\nMARK\nin 2\n",
        0,
    );
}

// ---- DIFF morphology (r81 mB/mC) ----

#[test]
fn mb1_break_marker_counts() {
    // Head firing: MARK + break (in-loop, silent); body skipped;
    // `echo after` firing: MARK + break at level 0 (one diagnostic).
    assert_case(
        "trap 'echo MARK; break' DEBUG; for i in 1 2 3; do echo \"in $i\"; done; echo after",
        "MARK\nMARK\nafter\n",
        1,
    );
}

#[test]
fn mb4_break_on_second_firing_still_runs_body_command() {
    // The 2nd firing is the body echo's own: break is set during it,
    // but the echo was already entered and prints; the loop then
    // exits at the post-body consumption.
    assert_case(
        "c=0; trap 'c=$((c+1)); echo \"fire $c\"; [ $c -eq 2 ] && break' DEBUG; for i in 1 2 3; do echo \"in $i\"; done; echo after",
        "fire 1\nfire 2\nin 1\nfire 3\nafter\n",
        0,
    );
}

#[test]
fn mb5_break_colon_body() {
    assert_case(
        "trap 'break' DEBUG; for i in 1 2 3; do :; done; echo after",
        "after\n",
        1,
    );
}

#[test]
fn mb6_break_echo_before_loop() {
    // Firings: `echo start` (level 0, diagnostic), for head (in-loop,
    // silent), `echo after` (level 0, diagnostic) = 2 diagnostics.
    assert_case(
        "trap 'break' DEBUG; echo start; for i in 1 2 3; do echo \"in $i\"; done; echo after",
        "start\nafter\n",
        2,
    );
}

#[test]
fn mb11_continue_marker_counts() {
    // Each head firing continues: bodies are all skipped, three MARKs
    // from the heads, the fourth from `echo after`'s firing, whose
    // continue is out-of-loop (one diagnostic).
    assert_case(
        "trap 'echo MARK; continue' DEBUG; for i in 1 2 3; do echo \"in $i\"; done; echo after",
        "MARK\nMARK\nMARK\nMARK\nafter\n",
        1,
    );
}

#[test]
fn mb12_break_nested_loop() {
    // The pending break skips the outer body's next command — the
    // whole inner for — via the entry gate; the outer post-body
    // consumption exits the only loop level the flag targeted.
    assert_case(
        "trap 'break' DEBUG; for i in 1 2; do for j in a b; do echo \"in $i$j\"; done; echo \"outer $i\"; done; echo after",
        "after\n",
        1,
    );
}

#[test]
fn mc2_bash_command_break_for() {
    assert_case(
        "trap 'echo \"DBG:[$BASH_COMMAND]\"; break' DEBUG; for i in 1 2; do echo \"in $i\"; done; echo after",
        "DBG:[for i in 1 2]\nDBG:[echo after]\nafter\n",
        1,
    );
}

#[test]
fn mc7_break_on_first_firing() {
    assert_case(
        "c=0; trap 'c=$((c+1)); echo \"fire $c\"; [ $c -eq 1 ] && break' DEBUG; for i in 1 2 3; do echo \"in $i\"; done; echo after",
        "fire 1\nfire 2\nafter\n",
        0,
    );
}

// ---- SAME morphology (positive controls) ----

#[test]
fn mb2_marker_only_control() {
    assert_case(
        "trap 'echo MARK' DEBUG; for i in 1 2 3; do echo \"in $i\"; done; echo after",
        "MARK\nMARK\nin 1\nMARK\nMARK\nin 2\nMARK\nMARK\nin 3\nMARK\nafter\n",
        0,
    );
}

#[test]
fn mb3_break_no_loop() {
    assert_case(
        "trap 'echo MARK; break' DEBUG; echo one; echo two",
        "MARK\none\nMARK\ntwo\n",
        2,
    );
}

#[test]
fn mb7_err_trap_break_unchanged() {
    // ERR trap actions are NOT deferred: the break unwinds from the
    // failed command's firing site exactly as before (both shells
    // print MARK once and skip the loop).
    assert_case(
        "trap 'echo MARK; break' ERR; for i in 1 2 3; do false; echo \"in $i\"; done; echo after",
        "MARK\nafter\n",
        0,
    );
}

#[test]
fn mb8_break_while_true() {
    assert_case(
        "trap 'break' DEBUG; while true; do echo hi; done; echo after",
        "after\n",
        1,
    );
}

#[test]
fn mb9_marker_plain_loop() {
    assert_case(
        "trap 'echo MARK' DEBUG; for i in 1 2 3; do echo \"in $i\"; done",
        "MARK\nMARK\nin 1\nMARK\nMARK\nin 2\nMARK\nMARK\nin 3\n",
        0,
    );
}

#[test]
fn mb10_break_in_if() {
    assert_case(
        "trap 'break' DEBUG; if true; then echo yes; fi; echo after",
        "yes\nafter\n",
        3,
    );
}

#[test]
fn mc1_bash_command_for() {
    assert_case(
        "trap 'echo \"DBG:[$BASH_COMMAND]\"' DEBUG; for i in 1 2; do echo \"in $i\"; done; echo after",
        "DBG:[for i in 1 2]\nDBG:[echo \"in $i\"]\nin 1\nDBG:[for i in 1 2]\nDBG:[echo \"in $i\"]\nin 2\nDBG:[echo after]\nafter\n",
        0,
    );
}

#[test]
fn mc3_while_break_on_second_firing() {
    // The 2nd firing is the while test's own; the test still runs,
    // the body is skipped by the entry gate, the loop exits at the
    // post-body consumption.
    assert_case(
        "c=0; trap 'c=$((c+1)); echo \"fire $c\"; [ $c -eq 2 ] && break' DEBUG; i=0; while [ $i -lt 3 ]; do i=$((i+1)); echo \"in $i\"; done; echo after",
        "fire 1\nfire 2\nfire 3\nafter\n",
        0,
    );
}

#[test]
fn mc5_marker_break_in_body() {
    // A plain (non-trap) break inside the body keeps the immediate
    // path; only the DEBUG firings interleave.
    assert_case(
        "trap 'echo MARK' DEBUG; for i in 1 2 3; do [ $i -eq 2 ] && break; echo \"in $i\"; done; echo after",
        "MARK\nMARK\nMARK\nin 1\nMARK\nMARK\nMARK\nMARK\nafter\n",
        0,
    );
}

#[test]
fn mc6_bash_command_while() {
    assert_case(
        "trap 'echo \"DBG:[$BASH_COMMAND]\"' DEBUG; i=0; while [ $i -lt 2 ]; do i=$((i+1)); echo \"in $i\"; done; echo after",
        "DBG:[i=0]\nDBG:[[ $i -lt 2 ]]\nDBG:[i=$((i+1))]\nDBG:[echo \"in $i\"]\nin 1\nDBG:[[ $i -lt 2 ]]\nDBG:[i=$((i+1))]\nDBG:[echo \"in $i\"]\nin 2\nDBG:[[ $i -lt 2 ]]\nDBG:[echo after]\nafter\n",
        0,
    );
}

#[test]
fn mc8_return_in_debug_trap() {
    // `return` in the action is untouched by the loop-control
    // deferral: five firings, five "can only return" diagnostics.
    let (stdout, stderr, code) = run_case(
        "trap 'echo MARK; return 0' DEBUG; for i in 1 2; do echo \"in $i\"; done; echo after",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "MARK\nMARK\nin 1\nMARK\nMARK\nin 2\nMARK\nafter\n");
    assert_eq!(
        stderr.matches("can only `return'").count(),
        5,
        "stderr: {stderr:?}"
    );
}
