//! rubash#1630 — async subshells and async builtins lost the inherited
//! trap table: the `rubash -c` respawn that runs `( ... ) &` bodies
//! stripped every `__RUBASH_TRAP*` env key, so the child seeded an
//! empty table and `trap -p` printed nothing. GNU's forked child lists
//! the inherited traps in reset form (dispositions reset — nothing
//! fires, the inherited EXIT trap never runs — but the LISTING is
//! preserved, and with it the `eval "$(trap -p ...)"` save/restore
//! idiom). The fix forwards the live table through the same
//! reset-for-subshell transform the in-process subshell path uses.
//!
//! Expectations verified against GNU bash 5.3.20 (oracle, script
//! files). Assertions use `contains` for the listing lines: the async
//! child's own entry ignores (SIGINT/SIGQUIT per setup_async_signals)
//! legitimately add `trap -- '' SIGINT`/`SIGQUIT` lines.

#![cfg(unix)]

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

#[test]
fn issue1630_async_subshell_lists_inherited_traps_in_reset_form() {
    let (stdout, _stderr, code) = rubash(
        "trap 'echo X' USR1; trap 'echo E' EXIT; trap '' USR2\n\
         ( trap -p ) & wait $!",
    );
    assert!(
        stdout.contains("trap -- 'echo X' SIGUSR1"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("trap -- 'echo E' EXIT"), "stdout: {stdout}");
    assert!(stdout.contains("trap -- '' SIGUSR2"), "stdout: {stdout}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1630_async_builtin_lists_inherited_trap() {
    let (stdout, _stderr, code) = rubash("trap 'echo X' USR1; trap -p USR1 & wait $!");
    assert!(
        stdout.contains("trap -- 'echo X' SIGUSR1"),
        "stdout: {stdout}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue1630_async_child_can_rearm_and_eval_restore() {
    // Listing first (inherited, reset), then a fresh arm inside the
    // child replaces it — the in-subshell re-arm of the flat path.
    let (stdout, _stderr, code) = rubash(
        "trap 'echo X' USR1\n\
         ( trap -p USR1; trap 'echo NEW' USR1; trap -p USR1 ) & wait $!",
    );
    assert!(
        stdout.contains("trap -- 'echo X' SIGUSR1"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("trap -- 'echo NEW' SIGUSR1"),
        "stdout: {stdout}"
    );
    assert_eq!(code, Some(0));

    // The save/restore idiom: eval of the printed form re-arms the
    // inherited action inside the child.
    let (stdout, _stderr, code) = rubash(
        "trap 'echo X' USR1\n\
         ( eval \"$(trap -p USR1)\"; trap -p USR1 ) & wait $!",
    );
    assert!(
        stdout.contains("trap -- 'echo X' SIGUSR1"),
        "stdout: {stdout}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue1630_forwarded_table_stays_reset_no_inherited_firing() {
    // The inherited EXIT trap must NOT fire when the async child
    // finishes — it fires exactly once, when the parent itself exits.
    let (stdout, _stderr, code) =
        rubash("trap 'echo EXITFIRED' EXIT; ( true ) & wait $!; echo done");
    assert!(stdout.contains("done\n"), "stdout: {stdout}");
    assert_eq!(stdout.matches("EXITFIRED").count(), 1, "stdout: {stdout}");
    assert_eq!(code, Some(0));

    // The inherited USR1 action must NOT fire in the child: its
    // disposition is reset to default, so a self-directed USR1 kills
    // the child (wait reports 128 + 10 = 138) and 'echo X' never runs.
    let (stdout, _stderr, code) = rubash(
        "trap 'echo X' USR1\n\
         ( kill -USR1 $BASHPID; echo NOT_REACHED ) & wait $!; echo wait_rc=$?",
    );
    assert!(!stdout.contains("echo X"), "stdout: {stdout}");
    assert!(!stdout.contains("X\n"), "stdout: {stdout}");
    assert!(!stdout.contains("NOT_REACHED"), "stdout: {stdout}");
    assert!(stdout.contains("wait_rc=138"), "stdout: {stdout}");
    assert_eq!(code, Some(0));
}
