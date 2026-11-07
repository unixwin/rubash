//! rubash#2991 — a command substitution executed inside a DEBUG trap
//! handler hangs the shell under `set -T` (functrace; bats' `set -eET`
//! is the field shape).
//!
//! GNU mechanism (bash 5.3 sources):
//! - trap.c:1263-1272 `run_debug_trap` fires only when the DEBUG trap
//!   is trapped and its `SIG_INPROGRESS` bit is clear; the bit is set
//!   for the whole execution of the trap action
//!   (`_run_trap_internal`). It is *process state*, not a call frame.
//! - A forked command-substitution child inherits the bit verbatim:
//!   `reset_signal` (trap.c:1398) clears only `SIG_TRAPPED`
//!   (`sigmodes[sig] &= ~SIG_TRAPPED; /* XXX - SIG_INPROGRESS? */`),
//!   and the nofork substitution path threads it explicitly —
//!   subst.c:7090 `was_trap = running_trap;` — into
//!   `parse_and_execute`. So while a handler runs, no command in any
//!   substitution it spawns can re-fire the DEBUG trap, not even a
//!   DEBUG trap armed inside the child itself.
//! - Under functrace the substitution child DOES keep the parent's
//!   DEBUG action live (reset_or_restore_signal_handlers, trap.c:1588
//!   via rubash's reset_for_subshell_inner), which is what makes the
//!   suppression bit load-bearing: outside a handler, firings inside
//!   a substitution body are normal functrace behaviour.
//!
//! Pre-fix rubash reset `debug_trap_running` to `false` in the
//! substitution fork copy (`command_substitution_executor`), so the
//! body's first command re-fired the handler, whose own substitution
//! forked another flag-cleared copy — unbounded re-entry (the process
//! allocated hundreds of MB and never finished; every case below hit
//! the harness timeout). `set -e` is not part of the trigger: `-T`
//! alone hangs, `-e` alone is fine.
//!
//! Every expectation below was measured against GNU bash 5.3.20.
//! All cases run as `rubash -c <script>` with LC_ALL=C and a bounded
//! wait, so a regression reports as a named timeout failure instead
//! of hanging the suite.

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
    let out = wait_bounded(&mut child, Duration::from_secs(10));
    assert!(!out.timed_out, "case timed out (hang): {script}");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.code,
    )
}

fn assert_case(script: &str, want_stdout: &str) {
    let (stdout, stderr, code) = run_case(script);
    assert_eq!(
        code,
        Some(0),
        "exit status for {script:?} (stderr: {stderr:?})"
    );
    assert_eq!(stdout, want_stdout, "stdout for {script:?}");
    assert_eq!(stderr, "", "stderr for {script:?}");
}

// ---- Hang shapes (fail-first: each hit the harness timeout) ----

#[test]
fn e_et_comsub_in_debug_handler_completes() {
    // The filed shape: bats runs `set -eET`; any handler comsub hung.
    assert_case(
        "set -eET; trap 'echo \"TRAP $(echo hi)\"' DEBUG; echo one; echo two",
        "TRAP hi\none\nTRAP hi\ntwo\n",
    );
}

#[test]
fn functrace_only_comsub_in_debug_handler_completes() {
    // Flag bisection: -T alone is the whole trigger.
    assert_case(
        "set -T; trap 'echo \"TRAP $(echo hi)\"' DEBUG; echo one; echo two",
        "TRAP hi\none\nTRAP hi\ntwo\n",
    );
}

#[test]
fn assignment_comsub_in_debug_handler_completes() {
    assert_case(
        "set -eET; trap 'v=$(true); echo \"TRAP[$v]\"' DEBUG; echo one; echo two",
        "TRAP[]\none\nTRAP[]\ntwo\n",
    );
}

#[test]
fn backquote_in_debug_handler_completes() {
    assert_case(
        "set -eET; trap 'echo \"TRAP `echo hi`\"' DEBUG; echo one; echo two",
        "TRAP hi\none\nTRAP hi\ntwo\n",
    );
}

#[test]
fn nested_comsub_in_debug_handler_completes() {
    assert_case(
        "set -T; trap 'echo \"TRAP $(echo $(echo deep))\"' DEBUG; echo one",
        "TRAP deep\none\n",
    );
}

#[test]
fn function_context_comsub_in_debug_handler_completes() {
    // Functrace also fires inside the function body; the handler's
    // substitution must stay suppressed there too (four firings:
    // definition, call, body command, final echo).
    assert_case(
        "set -T; trap 'echo \"TRAP $(echo hi)\"' DEBUG; f() { echo in-f; }; f; echo done",
        "TRAP hi\nTRAP hi\nTRAP hi\nin-f\nTRAP hi\ndone\n",
    );
}

#[test]
fn child_armed_debug_trap_stays_suppressed() {
    // GNU: the inherited in-progress bit suppresses even a DEBUG trap
    // armed inside the substitution child itself — the child body
    // runs to completion with no CHILDTRAP output at all.
    assert_case(
        "set -T; logtrap() { local v; v=$(trap 'echo CHILDTRAP' DEBUG; echo one; echo two); echo \"HANDLER[$v]\"; }; trap 'logtrap' DEBUG; echo main",
        "HANDLER[one\ntwo]\nmain\n",
    );
}

// ---- Controls (passed before the fix; must not move) ----

#[test]
fn errexit_only_comsub_in_debug_handler_control() {
    // Without functrace the child resets the DEBUG hook, so nothing
    // could re-fire even before the fix.
    assert_case(
        "set -e; trap 'echo \"TRAP $(echo hi)\"' DEBUG; echo one; echo two",
        "TRAP hi\none\nTRAP hi\ntwo\n",
    );
}

#[test]
fn handler_without_comsub_control() {
    assert_case(
        "set -eET; trap 'echo TRAP' DEBUG; echo one; echo two",
        "TRAP\none\nTRAP\ntwo\n",
    );
}

#[test]
fn functrace_comsub_outside_handler_still_fires() {
    // Over-suppression guard: OUTSIDE a handler, functrace still
    // inherits the DEBUG trap into a substitution body — the firing
    // for `echo inner` lands inside the capture itself.
    assert_case(
        "set -T; trap 'echo \"TRAP $BASH_COMMAND\"' DEBUG; x=$(echo inner); echo \"x=$x\"",
        "TRAP x=$(echo inner)\nTRAP echo \"x=$x\"\nx=TRAP echo inner\ninner\n",
    );
}

#[test]
fn child_loop_break_stays_immediate() {
    // The substitution child has no DEBUG-action call frame of its
    // own: a break in the child's own loop is a plain loop exit
    // (GNU prints `after`), not a pending trap flag.
    assert_case(
        "set -T; logtrap() { local v; v=$(for i in 1 2 3; do break; echo NO; done; echo after); echo \"HANDLER[$v]\"; }; trap 'logtrap' DEBUG; echo main",
        "HANDLER[after]\nmain\n",
    );
}
