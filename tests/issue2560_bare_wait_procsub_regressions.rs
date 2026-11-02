//! Bare `wait` settles the last process-substitution child when
//! it is $! (rubash#2560; GNU bash 5.3.20 oracle-verified
//! 2026-11-02).
//!
//! GNU wait_for_background_pids' tail (jobs.c:2853-2858): without
//! WAIT_WAITS_FOR_ALL_PROCSUBS (not defined in the oracle build),
//! a bare wait runs procsub_waitpid (jobs.c:1131) on
//! last_procsub_child ONLY when its pid equals
//! last_asynchronous_pid — blocking for that one consumer/
//! producer, never for earlier procsubs, and not at all when a
//! later background job took $!. Rubash tracked no last-procsub
//! pid and its bare wait only swept already-dead fork
//! placeholders, so a live `>(...)` consumer outlived the wait
//! and its output landed after the next command's.

#![cfg(unix)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn rubash_bin() -> String {
    env!("CARGO_BIN_EXE_rubash").to_string()
}

/// Run a script FILE under the built rubash; return (stdout, stderr).
/// `output()` reads stdout to EOF, so output a leaked (unwaited)
/// procsub prints after the shell exits is still captured, in
/// time order — that is what makes the leak shapes assertable.
fn run_script(tag: &str, body: &str) -> (String, String) {
    let dir = std::env::temp_dir().join(format!("issue2560-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    let path: PathBuf = dir.join("case.sh");
    fs::write(&path, body).expect("write script");
    let output = Command::new(rubash_bin())
        .arg(&path)
        .current_dir(&dir)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn issue2560_bare_wait_settles_output_procsub_consumer() {
    // The headline shape: the consumer is $!, so the bare wait
    // blocks for it — "inner" lands before "done".
    let (stdout, stderr) = run_script(
        "headline",
        "echo hi > >(sleep 0.3; echo inner); wait; echo done\n",
    );
    assert_eq!(stdout, "inner\ndone\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2560_bare_wait_settles_only_the_last_procsub() {
    // Two procsubs: only the LAST is $! and only it settles; the
    // first keeps running past the wait (and past `done`),
    // exactly like GNU without WAIT_WAITS_FOR_ALL_PROCSUBS.
    let (stdout, stderr) = run_script(
        "twoprocsub",
        "echo a > >(sleep 0.5; echo first); echo b > >(sleep 0.1; echo second); wait; echo done\n",
    );
    assert_eq!(stdout, "second\ndone\nfirst\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2560_bare_wait_skips_procsub_when_dollar_bang_moved_on() {
    // A later background job took $!: the gate
    // (last_procsub_child->pid == last_asynchronous_pid) fails,
    // so the bare wait waits the job only and the procsub leaks.
    let (stdout, stderr) = run_script(
        "bangmoved",
        "echo a > >(sleep 0.4; echo first); sleep 0.05 & wait; echo done\n",
    );
    assert_eq!(stdout, "done\nfirst\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2560_operand_wait_on_procsub_still_settles() {
    // Control: `wait $!` already settled a procsub consumer
    // before this fix and must keep doing so.
    let (stdout, stderr) = run_script(
        "operand",
        "echo hi > >(sleep 0.3; echo inner); wait $!; echo done\n",
    );
    assert_eq!(stdout, "inner\ndone\n");
    assert_eq!(stderr, "");
}
