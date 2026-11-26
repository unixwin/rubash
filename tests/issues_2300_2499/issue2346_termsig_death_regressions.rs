//! Fatal-signal death form (rubash#2346; GNU bash 5.3.20 oracle-verified
//! 2026-10-31, audit ch1 RC3).
//!
//! On an untrapped fatal signal, GNU runs the EXIT trap and then dies
//! BY the signal: sig.c:590 termsig_handler calls run_exit_trap() and
//! then sig.c:649-667 kill_shell(sig), which restores SIG_DFL and
//! re-raises. A parent inspecting the wait status therefore sees
//! WIFSIGNALED, not an exit with code 128+n. Rubash ran the trap (that
//! half was #1332) but then exited normally with 128+n for TERM, INT
//! and USR1; only SIGHUP took the kill_shell path (#2233). These pins
//! assert the wait-status form itself via ExitStatusExt::signal() —
//! the shell-level $? reads 128+n under both forms and cannot judge
//! this face.

#![cfg(unix)]

use std::fs;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::Command;

fn rubash_bin() -> String {
    env!("CARGO_BIN_EXE_rubash").to_string()
}

/// Run a script FILE under the built rubash; return
/// (stdout, stderr, exit code, killing signal).
fn run_script(tag: &str, body: &str) -> (String, String, Option<i32>, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("issue2346-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    let path: PathBuf = dir.join("case.sh");
    fs::write(&path, body).expect("write script");
    let output = Command::new(rubash_bin())
        .arg(&path)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
        output.status.signal(),
    )
}

fn run_dash_c(text: &str) -> (String, Option<i32>, Option<i32>) {
    let output = Command::new(rubash_bin())
        .arg("-c")
        .arg(text)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("run rubash -c");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.code(),
        output.status.signal(),
    )
}

#[test]
fn issue2346_term_toplevel_dies_by_signal() {
    let (stdout, _stderr, code, signal) = run_script(
        "term-top",
        "trap 'echo EXIT-TRAP' EXIT\nkill -TERM $$\necho unreachable\n",
    );
    assert_eq!(stdout, "EXIT-TRAP\n");
    assert_eq!(code, None, "must not exit normally: {stdout}");
    assert_eq!(signal, Some(libc::SIGTERM));
}

#[test]
fn issue2346_term_dash_c_dies_by_signal() {
    let (stdout, code, signal) = run_dash_c("trap 'echo EXIT-TRAP' EXIT; kill -TERM $$");
    assert_eq!(stdout, "EXIT-TRAP\n");
    assert_eq!(code, None);
    assert_eq!(signal, Some(libc::SIGTERM));
}

#[test]
fn issue2346_int_dies_by_signal() {
    let (stdout, _stderr, code, signal) = run_script(
        "int",
        "trap 'echo EXIT-TRAP' EXIT\nkill -INT $$\necho unreachable\n",
    );
    assert_eq!(stdout, "EXIT-TRAP\n");
    assert_eq!(code, None);
    assert_eq!(signal, Some(libc::SIGINT));
}

#[test]
fn issue2346_usr1_untrapped_dies_by_signal() {
    let (stdout, _stderr, code, signal) = run_script(
        "usr1",
        "trap 'echo EXIT-TRAP' EXIT\nkill -USR1 $$\necho unreachable\n",
    );
    assert_eq!(stdout, "EXIT-TRAP\n");
    assert_eq!(code, None);
    assert_eq!(signal, Some(libc::SIGUSR1));
}

#[test]
fn issue2346_hup_control_dies_by_signal() {
    // The path that already conformed before this fix (rubash#2233);
    // pinned so the generalization cannot regress it.
    let (stdout, _stderr, code, signal) = run_script(
        "hup",
        "trap 'echo EXIT-TRAP' EXIT\nkill -HUP $$\necho unreachable\n",
    );
    assert_eq!(stdout, "EXIT-TRAP\n");
    assert_eq!(code, None);
    assert_eq!(signal, Some(libc::SIGHUP));
}

#[test]
fn issue2346_term_during_external_wait_dies_by_signal() {
    // Audit m3e shape: a background subshell delivers TERM while the
    // shell is blocked waiting on a foreground external command.
    let (stdout, _stderr, code, signal) = run_script(
        "term-wait",
        "trap 'echo EXIT-TRAP' EXIT\nme=$$\n( sleep 0.3; kill -TERM \"$me\" ) &\nsleep 3\necho unreachable\n",
    );
    assert_eq!(stdout, "EXIT-TRAP\n");
    assert_eq!(code, None);
    assert_eq!(signal, Some(libc::SIGTERM));
}

#[test]
fn issue2346_term_during_wait_builtin_dies_by_signal() {
    let (stdout, _stderr, code, signal) = run_script(
        "term-wait-builtin",
        "trap 'echo EXIT-TRAP' EXIT\n( sleep 0.3; kill -TERM $$ ) &\nsleep 3 &\nwait\necho unreachable\n",
    );
    assert_eq!(stdout, "EXIT-TRAP\n");
    assert_eq!(code, None);
    assert_eq!(signal, Some(libc::SIGTERM));
}

#[test]
fn issue2346_exit_in_exit_trap_does_not_override_signal_death() {
    // GNU: kill_shell runs after run_exit_trap unconditionally — an
    // `exit` inside the EXIT trap does not convert the death into a
    // plain exit with that code.
    let (stdout, _stderr, code, signal) =
        run_script("trap-exit", "trap 'exit 5' EXIT\nkill -TERM $$\n");
    assert_eq!(stdout, "");
    assert_eq!(code, None, "exit 5 in the trap must not win: {code:?}");
    assert_eq!(signal, Some(libc::SIGTERM));
}

#[test]
fn issue2346_trapped_term_still_runs_handler_and_survives() {
    let (stdout, _stderr, code, signal) = run_script(
        "trapped",
        "trap 'echo GOT-TERM' TERM\nkill -TERM $$\necho alive\n",
    );
    assert_eq!(stdout, "GOT-TERM\nalive\n");
    assert_eq!(code, Some(0));
    assert_eq!(signal, None);
}

#[test]
fn issue2346_ignored_term_still_survives() {
    let (stdout, _stderr, code, signal) =
        run_script("ignored", "trap '' TERM\nkill -TERM $$\necho alive\n");
    assert_eq!(stdout, "alive\n");
    assert_eq!(code, Some(0));
    assert_eq!(signal, None);
}

#[test]
fn issue2346_no_exit_trap_term_still_dies_by_signal() {
    // No EXIT trap armed: the delivery is never caught and the kernel
    // default kills the shell in place — the form was already right;
    // pinned against the generalization disturbing it.
    let (stdout, _stderr, code, signal) = run_script("notrap", "kill -TERM $$\necho unreachable\n");
    assert_eq!(stdout, "");
    assert_eq!(code, None);
    assert_eq!(signal, Some(libc::SIGTERM));
}
