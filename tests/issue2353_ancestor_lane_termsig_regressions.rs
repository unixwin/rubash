//! Ancestor-lane fatal-signal death form (rubash#2353; GNU bash
//! 5.3.20 oracle-verified 2026-10-31).
//!
//! A #1654 routed delivery (`kill -SIG $$` inside a flat subshell,
//! addressed to the shell's own pid) is synthetic: no kernel
//! disposition ever fires for it, so the boundary dispatch enacts
//! the addressed shell's verdict. GNU's matrix for the untrapped
//! cell, from source:
//!   * no EXIT trap + non-interactive: GNU installs NO terminating-
//!     signal handler (sig.c:230-236 — initialize_terminating_
//!     signals runs for a non-interactive shell only when an EXIT
//!     trap is defined), so the delivery stays at SIG_DFL and the
//!     shell dies BY the signal. Rubash took a plain 128+n exit.
//!   * SIGINT is the exception: while a script shell waits for a
//!     foreground job, wait_sigint_handler (jobs.c:2908) records
//!     the delivery and otherwise ignores it; the shell dies of
//!     SIGINT only when the job itself died of it (jobs.c:4384,
//!     4410 child_caught_sigint). The lane's carrier completed
//!     normally by construction, so the delivery is absorbed.
//!   * trapped / ignored / EXIT-trap-armed cells are the older
//!     verdicts (controls below; the EXIT-trap cell is #2346's).
//! Death form is asserted via ExitStatusExt::signal() — shell $?
//! reads 128+n under both forms and cannot judge this face.

#![cfg(unix)]

use std::fs;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::Command;

fn rubash_bin() -> String {
    env!("CARGO_BIN_EXE_rubash").to_string()
}

/// Run a script FILE under the built rubash; return
/// (stdout, exit code, killing signal).
fn run_script(tag: &str, body: &str) -> (String, Option<i32>, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("issue2353-{tag}-{}", std::process::id()));
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

fn assert_dies_by(tag: &str, body: &str, sig: i32) {
    let (stdout, code, signal) = run_script(tag, body);
    assert_eq!(stdout, "", "{tag}: the script must not continue");
    assert_eq!(code, None, "{tag}: must not exit plainly: {stdout}");
    assert_eq!(signal, Some(sig), "{tag}: must die by signal {sig}");
}

#[test]
fn issue2353_ancestor_term_dies_by_signal() {
    assert_dies_by("term", "( kill -TERM $$ )\necho SURVIVED\n", libc::SIGTERM);
}

#[test]
fn issue2353_ancestor_hup_dies_by_signal() {
    assert_dies_by("hup", "( kill -HUP $$ )\necho SURVIVED\n", libc::SIGHUP);
}

#[test]
fn issue2353_ancestor_usr1_dies_by_signal() {
    assert_dies_by("usr1", "( kill -USR1 $$ )\necho SURVIVED\n", libc::SIGUSR1);
}

#[test]
fn issue2353_ancestor_usr2_dies_by_signal() {
    assert_dies_by("usr2", "( kill -USR2 $$ )\necho SURVIVED\n", libc::SIGUSR2);
}

#[test]
fn issue2353_ancestor_pipe_dies_by_signal() {
    assert_dies_by("pipe", "( kill -PIPE $$ )\necho SURVIVED\n", libc::SIGPIPE);
}

#[test]
fn issue2353_ancestor_alrm_dies_by_signal() {
    assert_dies_by("alrm", "( kill -ALRM $$ )\necho SURVIVED\n", libc::SIGALRM);
}

#[test]
fn issue2353_ancestor_xfsz_dies_by_signal() {
    assert_dies_by("xfsz", "( kill -XFSZ $$ )\necho SURVIVED\n", libc::SIGXFSZ);
}

#[test]
fn issue2353_ancestor_nested_term_dies_by_signal() {
    assert_dies_by(
        "nested",
        "( ( kill -TERM $$ ) )\necho SURVIVED\n",
        libc::SIGTERM,
    );
}

#[test]
fn issue2353_ancestor_term_before_further_commands() {
    // The verdict lands at the boundary: commands after the carrier
    // never run.
    assert_dies_by(
        "then-wait",
        "( kill -TERM $$ )\nsleep 0.2\necho SURVIVED\n",
        libc::SIGTERM,
    );
}

#[test]
fn issue2353_ancestor_trap_reset_term_dies_by_signal() {
    // `trap - TERM` restores the default disposition: the SIG_DFL
    // death applies again (and the earlier trap text never runs).
    assert_dies_by(
        "trap-reset",
        "trap 'echo T' TERM\ntrap - TERM\n( kill -TERM $$ )\necho SURVIVED\n",
        libc::SIGTERM,
    );
}

#[test]
fn issue2353_ancestor_int_is_absorbed() {
    // The wait-cooperative SIGINT cell: the carrier completed
    // normally, so the delivery is absorbed and the script runs on
    // with the carrier's status.
    let (stdout, code, signal) = run_script("int-absorbed", "( kill -INT $$ )\necho SURVIVED\n");
    assert_eq!(stdout, "SURVIVED\n");
    assert_eq!(code, Some(0));
    assert_eq!(signal, None);
}

#[test]
fn issue2353_ancestor_controls_unchanged() {
    // Trapped: the action runs at the boundary, the shell survives.
    let (stdout, code, signal) = run_script(
        "trapped",
        "trap 'echo TRAPPED' TERM\n( kill -TERM $$ )\necho SURVIVED\n",
    );
    assert_eq!(stdout, "TRAPPED\nSURVIVED\n");
    assert_eq!(code, Some(0));
    assert_eq!(signal, None);

    // Ignored: dropped, the shell survives.
    let (stdout, code, signal) = run_script(
        "ignored",
        "trap '' TERM\n( kill -TERM $$ )\necho SURVIVED\n",
    );
    assert_eq!(stdout, "SURVIVED\n");
    assert_eq!(code, Some(0));
    assert_eq!(signal, None);

    // SIGQUIT: bash ignores QUIT for itself (sig.c:333) — survives.
    let (stdout, code, signal) = run_script("quit", "( kill -QUIT $$ )\necho SURVIVED\n");
    assert_eq!(stdout, "SURVIVED\n");
    assert_eq!(code, Some(0));
    assert_eq!(signal, None);
}

#[test]
fn issue2353_ancestor_exit_trap_cell_unchanged() {
    // EXIT trap armed: #2346's verdict — trap runs, death by signal.
    let (stdout, code, signal) = run_script(
        "exit-trap",
        "trap 'echo EXIT-TRAP' EXIT\n( kill -TERM $$ )\necho SURVIVED\n",
    );
    assert_eq!(stdout, "EXIT-TRAP\n");
    assert_eq!(code, None);
    assert_eq!(signal, Some(libc::SIGTERM));
}
