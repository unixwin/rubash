//! rubash#546: trap semantics — the listing, in-action $BASH_COMMAND, and
//! the ERR+DEBUG rc combination.
//!
//! Three script-observable cells (WSL GNU 5.3.0(1) probes, 2026-10-12):
//!
//! cell 1 — a fresh shell's `trap` table is EMPTY: signals the shell
//!   ignores on its own behalf are not traps and never reach the listing.
//!   A signal ignored BEFORE startup (inherited SIG_IGN) IS listed —
//!   probe: a parent `trap '' RTMIN` makes the GNU child list
//!   `trap -- '' SIGRTMIN` — and rubash keeps that inherited-transport
//!   behavior via __RUBASH_TRAP_ORIG_IGN, which these tests strip to pin
//!   the fresh-shell contract.
//!
//! cell 2 — inside a trap action $BASH_COMMAND stays frozen at the value
//!   the trap fired with (variables.c:1558 get_bash_command reads
//!   the_printed_command_except_trap): an ERR action's DEBUG firings
//!   report the FAILED command, not the action's own commands.
//!
//! cell 3 — `trap false ERR; trap false DEBUG; echo x` leaves $? alone:
//!   `survived=0` (the trap actions' rc of 1 does not leak into the
//!   following command's status).

use std::process::Command;

fn run(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env_remove("__RUBASH_TRAP_ORIG_IGN")
        .env_remove("__RUBASH_TRAP_SIGRTMIN")
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

#[test]
fn fresh_shell_trap_listing_is_empty() {
    let (stdout, stderr, rc) = run("trap");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert_eq!(rc, Some(0));
}

#[test]
fn fresh_shell_trap_p_listing_is_empty() {
    let (stdout, stderr, rc) = run("trap -p");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert_eq!(rc, Some(0));
}

#[test]
fn explicit_traps_still_list() {
    let (stdout, stderr, rc) = run("trap 'echo bad' TERM; trap '' HUP; (trap)");
    // Subshell keeps the parent's table; the shell-internal SIGRTMIN
    // ignore leaks nothing.
    assert_eq!(stdout, "trap -- '' SIGHUP\ntrap -- 'echo bad' SIGTERM\n");
    assert_eq!(stderr, "");
    assert_eq!(rc, Some(0));
}

#[test]
fn inherited_ignore_marker_still_lists() {
    // A signal ignored BEFORE startup is a real inherited ignore (GNU
    // probe: parent `trap '' RTMIN` -> child lists `trap -- '' SIGRTMIN`);
    // rubash transports it in __RUBASH_TRAP_ORIG_IGN.
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("__RUBASH_TRAP_ORIG_IGN", "SIGUSR2")
        .arg("-c")
        .arg("trap")
        .output()
        .expect("run rubash");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "trap -- '' SIGUSR2\n"
    );
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn bash_command_stays_frozen_inside_err_action() {
    let script = "trap \"echo DEBUG: \\$BASH_COMMAND\" DEBUG\ntrap \"echo ACT; false\" ERR\nfalse\necho done=$?";
    let (stdout, _stderr, rc) = run(script);
    // GNU: the in-action DEBUG firings all report the failed command
    // `false`; the action's own `echo ACT` never becomes BASH_COMMAND.
    assert_eq!(
        stdout,
        "DEBUG: trap \"echo ACT; false\" ERR\nDEBUG: false\nDEBUG: false\nACT\nDEBUG: false\nDEBUG: echo done=$?\ndone=1\n",
        "rc={rc:?}"
    );
    assert_eq!(rc, Some(0));
}

#[test]
fn err_plus_debug_trap_rc_does_not_leak_into_status() {
    let (stdout, _stderr, rc) = run("trap false ERR; trap false DEBUG; echo x; echo survived=$?");
    assert_eq!(stdout, "x\nsurvived=0\n");
    assert_eq!(rc, Some(0));
}

#[test]
fn lone_err_trap_rc_still_does_not_leak() {
    let (stdout, _stderr, rc) = run("trap false ERR; echo x; echo survived=$?");
    assert_eq!(stdout, "x\nsurvived=0\n");
    assert_eq!(rc, Some(0));
}
