//! EXIT trap entry `$?` on a fatal-signal death (rubash#2352; GNU
//! bash 5.3.20 oracle-verified 2026-10-31).
//!
//! GNU runs the EXIT trap with trap_saved_exit_value =
//! last_command_exit_value (trap.c:1036 run_exit_trap), and nothing
//! on the termsig path (sig.c:590 termsig_handler -> sig.c:644
//! run_exit_trap -> sig.c:649-667 kill_shell) overwrites that value
//! with 128+sig first: the 128+sig synthesis belongs to CHILD wait
//! statuses (jobs.c:2961), never to the shell's own death. What the
//! value IS depends on how the delivery preempts execution:
//!   * a self-addressed kill builtin COMPLETES first (its own status,
//!     0), and the pending termsig is serviced at the next boundary —
//!     the trap sees the kill builtin's status;
//!   * a delivery that preempts a WAIT (external command, or the
//!     parent waiting on the delivering layer's construct) leaves the
//!     in-flight command uncommitted — the trap sees the last
//!     COMPLETED command's status.
//! Rubash ran the trap but fed it the 128+sig unwind status on every
//! lane (#2346 fixed the death FORM, not this status pipeline).
//! These pins assert the trap's observed status byte-for-byte plus
//! the wait-status form via ExitStatusExt::signal().

#![cfg(unix)]

use std::fs;
use std::io::Read;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn rubash_bin() -> String {
    env!("CARGO_BIN_EXE_rubash").to_string()
}

/// Run a script FILE under the built rubash; return
/// (stdout, exit code, killing signal).
fn run_script(tag: &str, body: &str) -> (String, Option<i32>, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("issue2352-{tag}-{}", std::process::id()));
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
        output.status.code(),
        output.status.signal(),
    )
}

#[test]
fn issue2352_term_direct_trap_sees_kill_builtin_status() {
    // The issue's canonical probe: GNU prints status=0 (the kill
    // builtin completed; its status is the last committed one).
    let (stdout, code, signal) =
        run_script("term-direct", "trap 'echo status=$?' EXIT\nkill -TERM $$\n");
    assert_eq!(stdout, "status=0\n");
    assert_eq!(code, None, "must die by the signal: {stdout}");
    assert_eq!(signal, Some(libc::SIGTERM));
}

#[test]
fn issue2352_hup_direct_trap_sees_kill_builtin_status() {
    // HUP face of the same pipeline: a status-setting command before
    // the kill does NOT show through — the kill's own 0 does.
    let (stdout, code, signal) = run_script(
        "hup-direct",
        "trap 'echo status=$?' EXIT\n(exit 3)\nkill -HUP $$\n",
    );
    assert_eq!(stdout, "status=0\n");
    assert_eq!(code, None);
    assert_eq!(signal, Some(libc::SIGHUP));
}

#[test]
fn issue2352_int_direct_trap_sees_kill_builtin_status() {
    let (stdout, code, signal) =
        run_script("int-direct", "trap 'echo status=$?' EXIT\nkill -INT $$\n");
    assert_eq!(stdout, "status=0\n");
    assert_eq!(code, None);
    assert_eq!(signal, Some(libc::SIGINT));
}

#[test]
fn issue2352_term_ancestor_trap_sees_pre_carrier_status() {
    // The parent is preempted while WAITING on the delivering
    // subshell: its status is never committed, so the trap sees the
    // last command completed before the carrier — `false` -> 1.
    let (stdout, code, signal) = run_script(
        "term-ancestor",
        "trap 'echo status=$?' EXIT\nfalse\n( kill -TERM $$ )\necho NEVER\n",
    );
    assert_eq!(stdout, "status=1\n");
    assert_eq!(code, None);
    assert_eq!(signal, Some(libc::SIGTERM));
}

#[test]
fn issue2352_hup_ancestor_trap_sees_pre_carrier_status() {
    let (stdout, code, signal) = run_script(
        "hup-ancestor",
        "trap 'echo status=$?' EXIT\n(exit 7)\n( kill -HUP $$ )\necho NEVER\n",
    );
    assert_eq!(stdout, "status=7\n");
    assert_eq!(code, None);
    assert_eq!(signal, Some(libc::SIGHUP));
}

#[test]
fn issue2352_term_during_external_wait_trap_sees_last_completed() {
    // An external TERM lands while the shell waits on `sleep`: the
    // in-flight command never commits, the trap sees `false` -> 1.
    let dir = std::env::temp_dir().join(format!("issue2352-wait-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    let path: PathBuf = dir.join("case.sh");
    fs::write(&path, "trap 'echo status=$?' EXIT\nfalse\nsleep 30\n").expect("write script");
    let mut child = Command::new(rubash_bin())
        .arg(&path)
        .env("LC_ALL", "C.UTF-8")
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    std::thread::sleep(std::time::Duration::from_millis(600));
    // SAFETY: kill(2) on the spawned child's pid.
    unsafe {
        libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
    }
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .expect("stdout pipe")
        .read_to_string(&mut stdout)
        .expect("read stdout");
    let status = child.wait().expect("wait for rubash");
    assert_eq!(stdout, "status=1\n");
    assert_eq!(status.code(), None);
    assert_eq!(status.signal(), Some(libc::SIGTERM));
}

#[test]
fn issue2352_trap_exit_status_does_not_replace_signal_death() {
    // GNU: the trap's own `exit 5` does not replace the death —
    // kill_shell follows run_exit_trap unconditionally (sig.c:646).
    let (stdout, code, signal) = run_script(
        "trap-exits",
        "trap 'echo status=$?; exit 5' EXIT\nkill -TERM $$\n",
    );
    assert_eq!(stdout, "status=0\n");
    assert_eq!(code, None);
    assert_eq!(signal, Some(libc::SIGTERM));
}

#[test]
fn issue2352_normal_exit_controls_unchanged() {
    // No signal: the trap keeps seeing the ordinary exit status.
    let (stdout, code, signal) = run_script("normal-false", "trap 'echo status=$?' EXIT\nfalse\n");
    assert_eq!(stdout, "status=1\n");
    assert_eq!(code, Some(1));
    assert_eq!(signal, None);

    let (stdout, code, signal) = run_script("normal-exit5", "trap 'echo status=$?' EXIT\nexit 5\n");
    assert_eq!(stdout, "status=5\n");
    assert_eq!(code, Some(5));
    assert_eq!(signal, None);
}
