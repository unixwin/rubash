//! Issues rubash#343/#344/#345 — trap firing order and inheritance.
//!
//! #343: `declare -ft f` (per-function trace attribute, GNU trace_p(var))
//! must make f inherit the RETURN trap with the global functrace option
//! OFF. GNU execute_cmd.c:5291-5295: "Shell functions inherit the RETURN
//! trap if function tracing is on globally or on individually for this
//! function" — the gate is `trace_p (var) == 0 && function_trace_mode ==
//! 0`, so either side alone keeps the trap; run_return_trap then fires at
//! the three function-exit sites (execute_cmd.c:5378/5399/5407).
//!
//! #344: the ERR trap fires for a failing command BEFORE errexit kills the
//! shell. GNU run_error_trap callers (execute_cmd.c:773/874/1000/1166) run
//! during command execution; the ERREXIT jump lives in
//! execute_command_internal (execute_cmd.c:624) after it — `set -e; trap
//! 'echo E' ERR; false` prints E and exits 1, never dying silently.
//!
//! #345: a DEBUG trap ARMED inside a command substitution fires for the
//! remaining commands of that body. GNU starts the substitution subshell
//! with the INHERITED DEBUG trap reset unless functrace is on (trap.c:1588
//! reset_or_restore_signal_handlers; the pre-existing functrace route in
//! command_substitution.rs covers that half), and a trap the body sets is
//! armed in the subshell itself (set_signal re-arms a reset disposition):
//! `echo "$(trap 'echo DC' DEBUG; echo x)"` prints DC then x.
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-28).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// #343: declare -ft makes the function inherit the RETURN trap; the exit
/// status of the body's `return` survives the trap (GNU: R then rc=5).
#[test]
fn trace_attribute_function_fires_return_trap() {
    let (stdout, stderr, code) =
        rubash("f() { return 5; }\ndeclare -ft f\ntrap 'echo R' RETURN\nf\necho \"rc=$?\"");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "R\nrc=5\n");
    assert_eq!(code, Some(0));
}

/// #343 guard: WITHOUT the attribute and without functrace, the RETURN trap
/// stays function-local (GNU prints only rc=5).
#[test]
fn untraced_function_does_not_fire_return_trap() {
    let (stdout, stderr, code) =
        rubash("f() { return 5; }\ntrap 'echo R' RETURN\nf\necho \"rc=$?\"");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "rc=5\n");
    assert_eq!(code, Some(0));
}

/// #343: the attribute also covers the DEBUG entry fire (execute_cmd.c:5387
/// gated on the same trace inheritance at 5270). GNU probe: the top-level
/// trap fires for the `f` invocation, the ft entry fire, the body's `:`,
/// and `echo after` — four D lines.
#[test]
fn trace_attribute_function_fires_debug_trap_at_entry() {
    let (stdout, stderr, code) =
        rubash("f() { :; }\ndeclare -ft f\ntrap 'echo D' DEBUG\nf\necho after");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "D\nD\nD\nD\nafter\n");
}

/// #344: ERR trap before errexit death.
#[test]
fn err_trap_fires_before_errexit_exits() {
    let (stdout, stderr, code) = rubash("set -e\ntrap 'echo E' ERR\nfalse\necho unreachable");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "E\n");
    assert_eq!(code, Some(1));
}

/// #344 guard: without errexit the ERR trap still fires exactly once and
/// execution continues.
#[test]
fn err_trap_alone_still_fires() {
    let (stdout, _, code) = rubash("trap 'echo E' ERR\nfalse\necho ok");
    assert_eq!(stdout, "E\nok\n");
    assert_eq!(code, Some(0));
}

/// #345: DEBUG trap armed inside a command substitution fires for the
/// body's later commands.
#[test]
fn debug_trap_armed_inside_comsub_fires() {
    let (stdout, stderr, code) = rubash(r#"echo "$(trap 'echo DC' DEBUG; echo x)""#);
    assert_eq!(stderr, "");
    assert_eq!(stdout, "DC\nx\n");
    assert_eq!(code, Some(0));
}

/// #345 guard: an INHERITED DEBUG trap does NOT fire inside the
/// substitution without functrace (GNU resets it at subshell start,
/// trap.c:1588) — only `x` reaches the output.
#[test]
fn inherited_debug_trap_stays_silent_in_comsub_without_functrace() {
    let (stdout, stderr, code) = rubash(r#"trap 'echo DC' DEBUG; echo "$(echo x)""#);
    assert_eq!(stderr, "");
    // The outer echo itself fires the trap once (top level, in scope); the
    // substitution body does not add more fires.
    assert_eq!(stdout, "DC\nx\n");
    assert_eq!(code, Some(0));
}

/// #345: with functrace the inherited DEBUG trap keeps firing inside the
/// substitution (execute_cmd.c subshell inheritance under
/// function_trace_mode).
#[test]
fn functrace_debug_trap_fires_inside_comsub() {
    let (stdout, stderr, code) =
        rubash(r#"set -o functrace; trap 'echo DC' DEBUG; echo "$(echo x)""#);
    assert_eq!(stderr, "");
    assert!(stdout.ends_with("x\n"), "stdout: {stdout}");
    assert!(stdout.matches("DC").count() >= 2, "stdout: {stdout}");
    assert_eq!(code, Some(0));
}

/// #345 guard: a plain subshell behaves the same way — inherited trap
/// silent, trap armed inside fires.
#[test]
fn subshell_debug_trap_armed_inside_fires() {
    let (stdout, stderr, code) = rubash("(trap 'echo DC' DEBUG; echo x)");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "DC\nx\n");
    assert_eq!(code, Some(0));
}
