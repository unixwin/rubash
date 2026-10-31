//! rubash#2348 + rubash#2347 — the two disposition holes on the
//! `trap - SIG` (REVERT) paths, found by the 1.5.0 pre-release audit
//! (chapter 1, RC5 + RC4) on master cb39c1d2.
//!
//! #2348 (RC5): after `trap - SIG`, re-setting a handler trap on the
//! SAME signal wrote the trap table but never reinstalled the OS
//! disposition, so the shell died (or stayed ignoring) on delivery.
//! Mechanism: signal-hook's Handle caches each signal's registration
//! and turns a later add_signal into a no-op, while reconcile's
//! Default/Ignore transitions overwrite the OS disposition with raw
//! sigaction calls — after a flip away, the backend slot stayed
//! registered but the kernel no longer routed deliveries to it.
//! GNU round-trips cleanly: the REVERT path (trap.c:1398
//! reset_signal) and the SET path (trap.c:838 set_signal, installing
//! trap_handler) compose. The fix makes ensure_handler detect the
//! desync and reinstall the captured backend dispatcher.
//!
//! #2347 (RC4): with SIGINT ignored on entry, `trap - INT` must
//! revert INT to the fatal termsig handler — the REVERT special case
//! at builtins/trap.def:228-251 (citing POSIX interp 751):
//! restore_default_signal early-returns on hard-ignored signals, yet
//! the builtin then installs the handler unconditionally for SIGINT.
//! Every other entry-ignored signal's reset stays a no-op, and
//! trap-set on entry-ignored signals stays a no-op (POSIX). The fix
//! adds the SIGINT-only revert in the trap builtin plus a reconcile
//! arm that keeps the seeded entry-ignore table entry from
//! re-imposing Ignore at every boundary.
//!
//! Expectations byte-verified against GNU bash 5.3.20 (script files
//! under both shells; death forms read via the parent wait status,
//! never `$?`). All pins are unix-only: the machinery under test is
//! the unix kernel-disposition layer.
#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

#[cfg(any(target_os = "linux", target_os = "android"))]
const SIGUSR1: i32 = 10;
#[cfg(not(any(target_os = "linux", target_os = "android")))]
const SIGUSR1: i32 = 30;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue2348-2347-{tag}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, body).expect("write fixture");
    path
}

struct Outcome {
    stdout: String,
    stderr: String,
    status: ExitStatus,
}

fn run_script(dir: &Path, script: &Path) -> Outcome {
    let output = Command::new(RUBASH)
        .arg(script)
        .current_dir(dir)
        .output()
        .expect("spawn rubash");
    Outcome {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        status: output.status,
    }
}

/// Run `script` in a shell whose entry disposition for `signal` is
/// SIG_IGN: a /bin/sh launcher sets the ignore and execs rubash, and
/// ignored dispositions survive exec — the real kernel entry-ignore,
/// not the transported table hint.
fn run_script_entry_ignored(dir: &Path, signal: &str, script: &Path) -> Outcome {
    let wrapper = write(
        dir,
        "ign-launcher.sh",
        &format!("#!/bin/sh\ntrap '' {signal}\nexec \"$RUBASH_UNDER_TEST\" \"$@\"\n"),
    );
    let output = Command::new("/bin/sh")
        .arg(&wrapper)
        .arg(script)
        .env("RUBASH_UNDER_TEST", RUBASH)
        .current_dir(dir)
        .output()
        .expect("spawn entry-ignored rubash");
    Outcome {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        status: output.status,
    }
}

fn case(tag: &str, body: &str) -> (PathBuf, PathBuf) {
    let dir = scratch(tag);
    let script = write(&dir, "case.sh", body);
    (dir, script)
}

// ---------------------------------------------------------------- #2348

#[test]
fn issue2348_clear_then_reset_same_signal_fires() {
    let (dir, script) = case(
        "m5a",
        "trap 'echo H1' USR1\ntrap - USR1\ntrap 'echo H2' USR1\nkill -USR1 $$\necho after\n",
    );
    let out = run_script(&dir, &script);
    assert_eq!(out.stderr, "");
    assert_eq!(out.stdout, "H2\nafter\n");
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn issue2348_clear_then_reset_term_fires() {
    let (dir, script) = case(
        "m5term",
        "trap 'echo H1' TERM\ntrap - TERM\ntrap 'echo H2' TERM\nkill -TERM $$\necho after\n",
    );
    let out = run_script(&dir, &script);
    assert_eq!(out.stdout, "H2\nafter\n");
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn issue2348_clear_then_reset_hup_fires() {
    let (dir, script) = case(
        "m5hup",
        "trap 'echo H1' HUP\ntrap - HUP\ntrap 'echo H2' HUP\nkill -HUP $$\necho after\n",
    );
    let out = run_script(&dir, &script);
    assert_eq!(out.stdout, "H2\nafter\n");
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn issue2348_clear_then_reset_sig_spelling_fires() {
    let (dir, script) = case(
        "m5spell",
        "trap 'echo H1' SIGUSR1\ntrap - SIGUSR1\ntrap 'echo H2' SIGUSR1\nkill -SIGUSR1 $$\necho after\n",
    );
    let out = run_script(&dir, &script);
    assert_eq!(out.stdout, "H2\nafter\n");
    assert_eq!(out.status.code(), Some(0));
}

/// The `trap -p` save/restore idiom (audit a7): the restored listing
/// was already byte-identical to GNU's; the restored disposition was
/// the hole. Restore via eval of the listing.
#[test]
fn issue2348_clear_then_reset_via_eval_of_trap_p_fires() {
    let (dir, script) = case(
        "m5eval",
        "trap 'echo HU' USR1\ntrap -p USR1 > saved.txt\ntrap - USR1\neval \"$(cat saved.txt)\"\nkill -USR1 $$\necho after\n",
    );
    let out = run_script(&dir, &script);
    assert_eq!(out.stdout, "HU\nafter\n");
    assert_eq!(out.status.code(), Some(0));
}

/// Same restore, via sourcing the saved listing file.
#[test]
fn issue2348_clear_then_reset_via_source_of_trap_p_fires() {
    let (dir, script) = case(
        "m5source",
        "trap 'echo HU' USR1\ntrap -p USR1 > saved.txt\ntrap - USR1\n. ./saved.txt\nkill -USR1 $$\necho after\n",
    );
    let out = run_script(&dir, &script);
    assert_eq!(out.stdout, "HU\nafter\n");
    assert_eq!(out.status.code(), Some(0));
}

/// The desync is not REVERT-specific: a `trap ''` flip between two
/// handler sets strands the second handler the same way (stuck at
/// SIG_IGN, handler never fires).
#[test]
fn issue2348_ignore_flip_between_handler_sets_still_fires() {
    let (dir, script) = case(
        "ignmid",
        "trap 'echo H1' USR1\ntrap '' USR1\ntrap 'echo H2' USR1\nkill -USR1 $$\necho after\n",
    );
    let out = run_script(&dir, &script);
    assert_eq!(out.stdout, "H2\nafter\n");
    assert_eq!(out.status.code(), Some(0));
}

// #2348 clean boundaries (must not regress):

#[test]
fn issue2348_overwrite_without_clear_fires_second() {
    let (dir, script) = case(
        "overwrite",
        "trap 'echo H1' USR1\ntrap 'echo H2' USR1\nkill -USR1 $$\necho after\n",
    );
    let out = run_script(&dir, &script);
    assert_eq!(out.stdout, "H2\nafter\n");
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn issue2348_clear_then_ignore_stays_ignored() {
    let (dir, script) = case(
        "clearign",
        "trap 'echo H1' USR1\ntrap - USR1\ntrap '' USR1\nkill -USR1 $$\necho after\n",
    );
    let out = run_script(&dir, &script);
    assert_eq!(out.stdout, "after\n");
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn issue2348_clear_then_different_signal_fires() {
    let (dir, script) = case(
        "diffsig",
        "trap 'echo H1' USR1\ntrap - USR1\ntrap 'echo H2' USR2\nkill -USR2 $$\necho after\n",
    );
    let out = run_script(&dir, &script);
    assert_eq!(out.stdout, "H2\nafter\n");
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn issue2348_clear_then_exit_trap_still_runs() {
    let (dir, script) = case(
        "exitafter",
        "trap 'echo H1' USR1\ntrap - USR1\ntrap 'echo EXITED' EXIT\necho body\n",
    );
    let out = run_script(&dir, &script);
    assert_eq!(out.stdout, "body\nEXITED\n");
    assert_eq!(out.status.code(), Some(0));
}
