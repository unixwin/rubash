//! rubash#3932: a plain-spawn child on a GNU nofork face exports
//! live-1, not the live level. GNU execute_disk_command decrements the
//! shell level and rebuilds the export environment before execve when
//! the command carries CMD_NO_FORK (execute_cmd.c:5813-5819,
//! adjust_shell_level variables.c:849-892); the mark's faces are the
//! consumed-string -c tail (evalstring.c:126 should_suppress_fork), the
//! user-subshell body's single simple command (execute_cmd.c:1756
//! optimize_subshell_command -> :1803), its connection tail
//! (optimize_connection_fork, execute_cmd.c:2886/2956), the process-
//! substitution body (should_suppress_fork with the PROCSUB bit,
//! evalstring.c:106-117) and the forked async child (execute_cmd.c:4553,
//! unconditional). Forked faces keep the live level: non-tail commands,
//! pipeline members, redirect tails (PROCSUB excepted), function tails,
//! compound-command tails, funsubs, script files.
//!
//! GNU oracle (vendored bash 5.3.20 @9c465866, WSL script-file probes,
//! env -i SHLVL=5 -> live 6):
//! - `( printenv SHLVL )` -> 5, `( true; printenv SHLVL )` -> 5,
//!   `( true && printenv SHLVL )` -> 5, `( ( printenv SHLVL ) )` -> 5,
//!   `! ( printenv SHLVL )` -> 5, `( printenv SHLVL ) > file` -> 5.
//! - `$( printenv SHLVL )` -> 5, `$( true; printenv SHLVL )` -> 5,
//!   backticks -> 5.
//! - `cat <( printenv SHLVL )` -> 5 (redirects ride: PROCSUB relaxes
//!   should_optimize_fork's redirect arm), `cat <( true; printenv
//!   SHLVL )` -> 5.
//! - `printenv SHLVL & wait` -> 5 (redirects ride: execute_cmd.c:4553
//!   marks unconditionally), `printenv SHLVL > file & wait` -> 5,
//!   `( printenv SHLVL ) & wait` -> 5, `f() ( printenv SHLVL ); f &
//!   wait` via the paren face -> 5.
//! - live-level guards: `( printenv SHLVL; true )` -> 6, `( printenv
//!   SHLVL | cat )` -> 6, `( printenv SHLVL > file )` -> 6,
//!   `( ! printenv SHLVL )` -> 6, `time ( printenv SHLVL )` -> 6,
//!   `f() { printenv SHLVL; }; ( f )` -> 6, `{ printenv SHLVL; } &
//!   wait` -> 6, `printenv SHLVL | cat & wait` -> 6,
//!   `${ printenv SHLVL; }` -> 6, `$( printenv SHLVL > file )` -> 6.
//! - trap gates: a trap armed inside the body blocks (`( trap 'true'
//!   EXIT; printenv SHLVL )` -> 6, ERR likewise), a parent-set trap
//!   does not (the forked child's reset, trap.c:1480
//!   reset_signal_handlers -> reset_or_restore_signal_handlers clears
//!   the dispositions): `( printenv SHLVL )` under `trap 'true' EXIT`
//!   -> 5; the async face is unconditional (execute_cmd.c:4553).
//! - the adjustment reads the CURRENT variable where the marked command
//!   runs: `SHLVL=9 ( printenv SHLVL )` -> 8, `( SHLVL=3; printenv
//!   SHLVL )` -> 2, `SHLVL=9 printenv SHLVL & wait` -> 8; absent SHLVL
//!   clamps at 0 (variables.c:856 counts absent as 0, :860-861 clamps).
//!
//! Harness note: extra_env SHLVL=5 means the shell runs live at 6
//! (initialize_shell_level, variables.c:894-897); `printenv` is a plain
//! external so the child env read is the spawn's exported level.

#![cfg(unix)]

use std::process::Command;

fn rubash(script: &str, extra_env: &[(&str, &str)]) -> (String, String, Option<i32>) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .arg("-c")
        .arg(script)
        .env_clear()
        .env("PATH", "/usr/bin:/bin");
    for (name, value) in extra_env {
        command.env(name, value);
    }
    let output = command.output().expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// `printenv SHLVL` prints the exported level bare; compare the whole
/// trimmed stdout (the expected strings below are that level, with the
/// probe text the script itself adds).
fn assert_child(script: &str, expected: &str) {
    let (stdout, stderr, code) = rubash(script, &[("SHLVL", "5")]);
    assert_eq!(code, Some(0), "script: {script} stderr: {stderr}");
    assert_eq!(stderr, "", "script: {script}");
    assert_eq!(stdout.trim(), expected, "script: {script}");
}

// ---- paren-subshell faces (execute_cmd.c:1756/1803) ----

#[test]
fn issue3932_paren_single_exports_live_minus_one() {
    assert_child("( /usr/bin/printenv SHLVL )", "5");
}

#[test]
fn issue3932_paren_semi_tail_exports_live_minus_one() {
    assert_child("( true; /usr/bin/printenv SHLVL )", "5");
}

#[test]
fn issue3932_paren_and_tail_exports_live_minus_one() {
    assert_child("( true && /usr/bin/printenv SHLVL )", "5");
}

#[test]
fn issue3932_nested_paren_exports_live_minus_one() {
    assert_child("( ( /usr/bin/printenv SHLVL ) )", "5");
}

#[test]
fn issue3932_inverted_paren_exports_live_minus_one() {
    // CMD_INVERT_RETURN is cleared from the subshell before the mark
    // (execute_cmd.c:1581-1583); the inverted status (1) is expected.
    let (stdout, stderr, code) = rubash("! ( /usr/bin/printenv SHLVL )", &[("SHLVL", "5")]);
    assert_eq!(code, Some(1), "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(stdout.trim(), "5");
}

#[test]
fn issue3932_paren_outer_redirect_exports_live_minus_one() {
    // the redirect belongs to the subshell, not the marked inner command
    let (stdout, _stderr, code) = rubash(
        "( /usr/bin/printenv SHLVL ) >/tmp/.i3932a; cat /tmp/.i3932a; rm -f /tmp/.i3932a",
        &[("SHLVL", "5")],
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "5");
}

#[test]
fn issue3932_paren_multi_nontail_keeps_live() {
    assert_child("( /usr/bin/printenv SHLVL; true )", "6");
}

#[test]
fn issue3932_paren_pipeline_keeps_live() {
    assert_child("( /usr/bin/printenv SHLVL | cat )", "6");
}

#[test]
fn issue3932_paren_redirect_tail_keeps_live() {
    assert_child("( /usr/bin/printenv SHLVL >/dev/null ); echo done", "done");
}

#[test]
fn issue3932_paren_inverted_inner_keeps_live() {
    assert_child("( ! /usr/bin/printenv SHLVL; echo done )", "6\ndone");
}

#[test]
fn issue3932_time_paren_keeps_live() {
    // CMD_TIME_PIPELINE blocks optimize_subshell_command
    // (execute_cmd.c:1755). GNU writes the time report to the shell's
    // stderr, which the body's own `2>/dev/null` does not silence here
    // (a report-plumbing divergence outside this issue's scope), so
    // only stdout is asserted.
    let (stdout, _stderr, code) = rubash(
        "time ( /usr/bin/printenv SHLVL ) 2>/dev/null",
        &[("SHLVL", "5")],
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "6");
}

#[test]
fn issue3932_paren_function_body_keeps_live() {
    let (stdout, _stderr, code) =
        rubash("f() { /usr/bin/printenv SHLVL; }; ( f )", &[("SHLVL", "5")]);
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "6");
}

#[test]
fn issue3932_paren_trap_inside_blocks() {
    assert_child("( trap 'true' EXIT; /usr/bin/printenv SHLVL )", "6");
}

#[test]
fn issue3932_paren_trap_ignore_exit_blocks() {
    assert_child("( trap '' EXIT; /usr/bin/printenv SHLVL )", "6");
}

#[test]
fn issue3932_paren_err_trap_inside_blocks() {
    assert_child("( trap 'true' ERR; false; /usr/bin/printenv SHLVL )", "6");
}

#[test]
fn issue3932_paren_parent_trap_does_not_block() {
    // the forked child's trap dispositions are reset
    // (trap.c:1480 reset_signal_handlers)
    assert_child("trap 'true' EXIT; ( /usr/bin/printenv SHLVL )", "5");
}

// ---- comsub faces (evalstring.c:532 should_suppress_fork) ----

#[test]
fn issue3932_comsub_single_exports_live_minus_one() {
    assert_child("echo child=$( /usr/bin/printenv SHLVL )", "child=5");
}

#[test]
fn issue3932_backtick_exports_live_minus_one() {
    assert_child("echo child=`/usr/bin/printenv SHLVL`", "child=5");
}

#[test]
fn issue3932_comsub_list_tail_exports_live_minus_one() {
    assert_child("echo child=$( true; /usr/bin/printenv SHLVL )", "child=5");
}

#[test]
fn issue3932_comsub_and_tail_exports_live_minus_one() {
    assert_child("echo child=$( true && /usr/bin/printenv SHLVL )", "child=5");
}

#[test]
fn issue3932_comsub_redirect_tail_keeps_live() {
    let (stdout, _stderr, code) = rubash(
        "echo child=$( /usr/bin/printenv SHLVL >/tmp/.i3932b ); cat /tmp/.i3932b; rm -f /tmp/.i3932b",
        &[("SHLVL", "5")],
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "child=\n6");
}

#[test]
fn issue3932_funsub_keeps_live() {
    assert_child("echo child=${ /usr/bin/printenv SHLVL; }", "child=6");
}

// ---- procsub faces (should_suppress_fork with the PROCSUB bit) ----

#[test]
fn issue3932_procsub_single_exports_live_minus_one() {
    let (stdout, _stderr, code) = rubash("cat <( /usr/bin/printenv SHLVL )", &[("SHLVL", "5")]);
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "5");
}

#[test]
fn issue3932_procsub_redirect_exports_live_minus_one() {
    let (stdout, _stderr, code) = rubash(
        "cat <( /usr/bin/printenv SHLVL >/tmp/.i3932c ); cat /tmp/.i3932c; rm -f /tmp/.i3932c",
        &[("SHLVL", "5")],
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "5");
}

#[test]
fn issue3932_procsub_conn_tail_exports_live_minus_one() {
    let (stdout, _stderr, code) =
        rubash("cat <( true; /usr/bin/printenv SHLVL )", &[("SHLVL", "5")]);
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "5");
}

// ---- async faces (execute_cmd.c:4553 unconditional) ----

#[test]
fn issue3932_async_exports_live_minus_one() {
    assert_child("/usr/bin/printenv SHLVL & wait", "5");
}

#[test]
fn issue3932_async_redirect_exports_live_minus_one() {
    let (stdout, _stderr, code) = rubash(
        "/usr/bin/printenv SHLVL >/tmp/.i3932d & wait; cat /tmp/.i3932d; rm -f /tmp/.i3932d",
        &[("SHLVL", "5")],
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "5");
}

#[test]
fn issue3932_async_pipeline_keeps_live() {
    assert_child("/usr/bin/printenv SHLVL | cat & wait; echo done", "6\ndone");
}

#[test]
fn issue3932_async_paren_exports_live_minus_one() {
    assert_child("( /usr/bin/printenv SHLVL ) & wait", "5");
}

#[test]
fn issue3932_async_subshell_conn_exports_live_minus_one() {
    assert_child("( true; /usr/bin/printenv SHLVL ) & wait", "5");
}

#[test]
fn issue3932_async_group_keeps_live() {
    assert_child("{ /usr/bin/printenv SHLVL; } & wait", "6");
}

#[test]
fn issue3932_async_parent_trap_does_not_block() {
    assert_child("trap 'true' EXIT; /usr/bin/printenv SHLVL & wait", "5");
}

// ---- the adjustment reads the CURRENT variable, clamps at 0 ----

#[test]
fn issue3932_prefix_shlvl_visible_to_paren_adjustment() {
    assert_child("( SHLVL=9 /usr/bin/printenv SHLVL )", "8");
}

#[test]
fn issue3932_body_assignment_visible_to_paren_adjustment() {
    assert_child("( SHLVL=3; /usr/bin/printenv SHLVL )", "2");
}

#[test]
fn issue3932_prefix_shlvl_visible_to_async_adjustment() {
    assert_child("SHLVL=9 /usr/bin/printenv SHLVL & wait", "8");
}

#[test]
fn issue3932_absent_shlvl_clamps_at_zero() {
    let (stdout, _stderr, code) = rubash("( /usr/bin/printenv SHLVL )", &[]);
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "0");
}

#[test]
fn issue3932_zero_shlvl_clamps_at_zero() {
    let (stdout, _stderr, code) = rubash("( /usr/bin/printenv SHLVL )", &[("SHLVL", "0")]);
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "0");
}

#[test]
fn issue3932_unexported_shlvl_still_exports_adjusted() {
    // adjust_shell_level rebinds + auto-exports (variables.c:889
    // bind_variable); the 3310 suite pins the exec face, this pins the
    // paren face.
    let (stdout, _stderr, code) = rubash(
        "export -n SHLVL; ( /usr/bin/printenv SHLVL )",
        &[("SHLVL", "5")],
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "5");
}
