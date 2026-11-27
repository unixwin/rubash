//! rubash#2546: `set -eu` + armed DEBUG/EXIT traps + a nounset failure
//! inside an untraced function must not print a DEBUG record for the
//! failing command. GNU's EXIT action runs in the DYING scope: an
//! untraced function disarmed the DEBUG trap at entry
//! (execute_cmd.c:5278 restore_default_signal), the fatal -e unwind
//! (ExitCode longjmp) skips every execute_function unwind protect —
//! uw_maybe_set_debug_trap included (execute_cmd.c:5276, with
//! uw_maybe_set_error_trap at :5286 and uw_maybe_set_return_trap at
//! :5301) — so the trap stays disarmed while the EXIT action executes
//! and run_debug_trap's SIG_TRAPPED gate (trap.c:1275) never fires.
//! #2506 pinned the armed half (top-level failure / functrace:
//! re-presentation firing, frozen BASH_COMMAND) but rubash's function
//! epilogue restored the saved DEBUG action on the unwind path,
//! re-arming the trap before the EXIT funnel — one extra
//! `DEBUG cmd=[echo ${u2482bad}]` per dying untraced frame chain.
//!
//! The "+0 counting law": for every cell of
//! {success, failure} x {function, top-level} x {DEBUG, EXIT, RETURN}
//! (x functrace, x -e off) the number of DEBUG lines matches GNU
//! byte-for-byte — failures inside an untraced function add ZERO
//! DEBUG lines, while failures at the top level / inside traced
//! functions keep the re-presentation firing (#2506's pinned cells,
//! guarded here against over-correction).
//!
//! Expectations below are GNU bash 5.3.20 outputs, probed from script
//! files on 2026-11-02 (WSL /usr/local/bin/bash).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn rubash_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .env_remove("WINUXSH_ROOT")
        .env_remove("__RUBASH_SHELL_ROOT");
    command
}

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2546-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn run_script(dir: &Path, name: &str, body: &str) -> std::process::Output {
    let script = dir.join(name);
    fs::write(&script, body).expect("write script");
    rubash_command()
        .current_dir(dir)
        .arg(script.to_string_lossy().replace('\\', "/"))
        .output()
        .expect("run script")
}

/// The +0 counting law: a DEBUG line fires once per GNU firing —
/// count them so a byte-diff can't hide a count change.
fn debug_line_count(stdout: &str) -> usize {
    stdout
        .lines()
        .filter(|line| line.starts_with("DEBUG cmd=[") || line.starts_with("D["))
        .count()
}

// 01 — the issue's first form (p2482b): function-valued EXIT action.
// GNU: exactly two DEBUG records (the EXIT arming command and the
// call `f`); the failing `echo ${u2482bad}` inside the untraced
// function gets NONE — master printed one extra.
#[test]
fn issue2546_01_function_exit_action_no_extra_debug() {
    let dir = test_dir("01");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eu\n\
         dbg() { echo \"DEBUG cmd=[$BASH_COMMAND]\"; }\n\
         xt() { echo \"EXIT action\"; }\n\
         trap dbg DEBUG; trap xt EXIT\n\
         f() { echo \"pre\"; echo ${u2482bad}; }\n\
         f\n\
         echo \"unreachable\"\n",
    );
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout,
        "DEBUG cmd=[trap xt EXIT]\n\
         DEBUG cmd=[f]\n\
         pre\n\
         EXIT action\n"
    );
    assert_eq!(debug_line_count(&stdout), 2);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("u2482bad: unbound variable"),
        "stderr: {stderr}"
    );
    let _ = fs::remove_dir_all(&dir);
}

// 02 — the issue's second form (p2482c): string-valued EXIT action.
// Same law: no DEBUG record for the failing command.
#[test]
fn issue2546_02_string_exit_action_no_extra_debug() {
    let dir = test_dir("02");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eu\n\
         dbg() { echo \"DEBUG cmd=[$BASH_COMMAND]\"; }\n\
         trap dbg DEBUG\n\
         trap 'echo EXIT-string' EXIT\n\
         f() { echo \"pre\"; echo ${u2482bad}; }\n\
         f\n\
         echo \"unreachable\"\n",
    );
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout,
        "DEBUG cmd=[trap 'echo EXIT-string' EXIT]\n\
         DEBUG cmd=[f]\n\
         pre\n\
         EXIT-string\n"
    );
    assert_eq!(debug_line_count(&stdout), 2);
    let _ = fs::remove_dir_all(&dir);
}

// 03 — failure inside the function with NO EXIT trap: still zero
// DEBUG records from the body (the unwind itself never fires).
#[test]
fn issue2546_03_failure_in_function_debug_only() {
    let dir = test_dir("03");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eu\n\
         dbg() { echo \"D[$BASH_COMMAND]\"; }\n\
         trap dbg DEBUG\n\
         f() { echo pre; echo ${u2482bad}; }\n\
         f\n\
         echo unreachable\n",
    );
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout, "D[f]\npre\n");
    assert_eq!(debug_line_count(&stdout), 1);
    let _ = fs::remove_dir_all(&dir);
}

// 04 — failure at the TOP level (armed scope, #2506's pinned shape):
// the failing command takes its normal pre-command firing AND the
// EXIT action's first command takes the re-presentation firing —
// two records, not three. Guards against over-correcting the fix.
#[test]
fn issue2546_04_top_level_failure_keeps_representation() {
    let dir = test_dir("04");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eu\n\
         dbg() { echo \"D[$BASH_COMMAND]\"; }\n\
         trap dbg DEBUG\n\
         trap 'echo XIT' EXIT\n\
         echo ${u2482bad}\n\
         echo unreachable\n",
    );
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout,
        "D[trap 'echo XIT' EXIT]\n\
         D[echo ${u2482bad}]\n\
         D[echo ${u2482bad}]\n\
         XIT\n"
    );
    assert_eq!(debug_line_count(&stdout), 3);
    let _ = fs::remove_dir_all(&dir);
}

// 05 — failure at the top level, no EXIT trap: exactly one DEBUG
// record (the failing command's own pre-command firing).
#[test]
fn issue2546_05_top_level_failure_debug_only() {
    let dir = test_dir("05");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eu\n\
         dbg() { echo \"D[$BASH_COMMAND]\"; }\n\
         trap dbg DEBUG\n\
         echo ${u2482bad}\n\
         echo unreachable\n",
    );
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout, "D[echo ${u2482bad}]\n");
    assert_eq!(debug_line_count(&stdout), 1);
    let _ = fs::remove_dir_all(&dir);
}

// 06 — failure in the function with RETURN armed too: RETURN never
// fires (untraced body, fatal unwind), DEBUG stays at two records.
#[test]
fn issue2546_06_failure_in_function_return_armed() {
    let dir = test_dir("06");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eu\n\
         dbg() { echo \"D[$BASH_COMMAND]\"; }\n\
         trap dbg DEBUG\n\
         trap 'echo XIT' EXIT\n\
         trap 'echo RET fn=${FUNCNAME[0]:-top}' RETURN\n\
         f() { echo pre; echo ${u2482bad}; }\n\
         f\n\
         echo unreachable\n",
    );
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout,
        "D[trap 'echo XIT' EXIT]\n\
         D[trap 'echo RET fn=${FUNCNAME[0]:-top}' RETURN]\n\
         D[f]\n\
         pre\n\
         XIT\n"
    );
    assert_eq!(debug_line_count(&stdout), 3);
    let _ = fs::remove_dir_all(&dir);
}

// 07 — success in the function with RETURN armed (no EXIT): the
// EXIT-less run fires DEBUG per command only; no RET record at the
// script end for a main (non-sourced) script.
#[test]
fn issue2546_07_success_in_function_return_armed() {
    let dir = test_dir("07");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eu\n\
         dbg() { echo \"D[$BASH_COMMAND]\"; }\n\
         trap dbg DEBUG\n\
         trap 'echo RET fn=${FUNCNAME[0]:-top}' RETURN\n\
         f() { echo pre; }\n\
         f\n\
         echo after\n",
    );
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout,
        "D[trap 'echo RET fn=${FUNCNAME[0]:-top}' RETURN]\n\
         D[f]\n\
         pre\n\
         D[echo after]\n\
         after\n"
    );
    assert_eq!(debug_line_count(&stdout), 3);
    let _ = fs::remove_dir_all(&dir);
}

// 08 — functrace ON: the function inherits DEBUG, so the failing
// command fires, and the EXIT action keeps the re-presentation
// firing — six records, none of which the fix may drop.
#[test]
fn issue2546_08_functrace_failure_keeps_all_firings() {
    let dir = test_dir("08");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eu\n\
         set -o functrace\n\
         dbg() { echo \"D[$BASH_COMMAND]\"; }\n\
         trap dbg DEBUG\n\
         trap 'echo XIT' EXIT\n\
         f() { echo pre; echo ${u2482bad}; }\n\
         f\n\
         echo unreachable\n",
    );
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout,
        "D[trap 'echo XIT' EXIT]\n\
         D[f]\n\
         D[f]\n\
         D[echo pre]\n\
         pre\n\
         D[echo ${u2482bad}]\n\
         D[echo ${u2482bad}]\n\
         XIT\n"
    );
    assert_eq!(debug_line_count(&stdout), 6);
    let _ = fs::remove_dir_all(&dir);
}

// 09 — `-e` OFF (`set -u` only): the nounset DISCARD unwind runs the
// protects first (sig.c:387 top_level_cleanup -> run_unwind_protects),
// the DEBUG trap is RESTORED before the EXIT trap, and the action's
// first command takes exactly one re-presentation firing — the same
// single record master wrongly added under `-e` on.
#[test]
fn issue2546_09_no_errexit_discard_restores_trap_before_exit() {
    let dir = test_dir("09");
    let output = run_script(
        &dir,
        "case.sh",
        "set -u\n\
         dbg() { echo \"D[$BASH_COMMAND]\"; }\n\
         trap dbg DEBUG\n\
         trap 'echo XIT' EXIT\n\
         f() { echo pre; echo ${u2482bad}; }\n\
         f\n\
         echo unreachable\n",
    );
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout,
        "D[trap 'echo XIT' EXIT]\n\
         D[f]\n\
         pre\n\
         D[echo ${u2482bad}]\n\
         XIT\n"
    );
    assert_eq!(debug_line_count(&stdout), 3);
    let _ = fs::remove_dir_all(&dir);
}

// 10 — nested untraced functions (g -> f, failure in f): every dying
// frame's protect is skipped, so the EXIT action still takes ZERO
// DEBUG firings.
#[test]
fn issue2546_10_nested_failure_no_extra_debug() {
    let dir = test_dir("10");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eu\n\
         dbg() { echo \"D[$BASH_COMMAND]\"; }\n\
         trap dbg DEBUG\n\
         trap 'echo XIT' EXIT\n\
         f() { echo finner; echo ${u2482bad}; }\n\
         g() { echo gpre; f; }\n\
         g\n\
         echo unreachable\n",
    );
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout,
        "D[trap 'echo XIT' EXIT]\n\
         D[g]\n\
         gpre\n\
         finner\n\
         XIT\n"
    );
    assert_eq!(debug_line_count(&stdout), 2);
    let _ = fs::remove_dir_all(&dir);
}

// 11 — plain errexit failure (`false`, no nounset): same law — the
// failing command inside the untraced function gets no DEBUG record
// and the EXIT action adds none.
#[test]
fn issue2546_11_errexit_false_failure_no_extra_debug() {
    let dir = test_dir("11");
    let output = run_script(
        &dir,
        "case.sh",
        "set -e\n\
         dbg() { echo \"D[$BASH_COMMAND]\"; }\n\
         trap dbg DEBUG\n\
         trap 'echo XIT' EXIT\n\
         f() { echo pre; false; }\n\
         f\n\
         echo unreachable\n",
    );
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout,
        "D[trap 'echo XIT' EXIT]\n\
         D[f]\n\
         pre\n\
         XIT\n"
    );
    assert_eq!(debug_line_count(&stdout), 2);
    let _ = fs::remove_dir_all(&dir);
}

// 12 — success baseline with EXIT armed: the EXIT action's first
// command takes its re-presentation firing wearing the frozen
// BASH_COMMAND (`echo after` twice), exactly as GNU (#2506's
// issue2212_13 shape) — the fix must not silence the armed window.
#[test]
fn issue2546_12_success_baseline_keeps_exit_representation() {
    let dir = test_dir("12");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eu\n\
         dbg() { echo \"D[$BASH_COMMAND]\"; }\n\
         trap dbg DEBUG\n\
         trap 'echo XIT' EXIT\n\
         f() { echo pre; }\n\
         f\n\
         echo after\n",
    );
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout,
        "D[trap 'echo XIT' EXIT]\n\
         D[f]\n\
         pre\n\
         D[echo after]\n\
         after\n\
         D[echo after]\n\
         XIT\n"
    );
    assert_eq!(debug_line_count(&stdout), 4);
    let _ = fs::remove_dir_all(&dir);
}
