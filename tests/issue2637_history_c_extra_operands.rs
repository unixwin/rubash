//! rubash#2637 — `history -c` with surplus operands must silently clear
//! the list and return 0, like GNU.
//!
//! GNU mechanism (builtins/history.def history_builtin): the CFLAG arm
//! clears the list first and returns EXECUTION_SUCCESS at once when no
//! words remain after getopt — so a `-d` that consumed its operand never
//! runs and its offset is never validated (`-c -d 1`, `-c -d bad`).
//! With words remaining and no other mode flag, execution falls through
//! past the display branch (excluded by CFLAG), so the operands are
//! ignored entirely — never validated as a number, never listed
//! (`-c extra`, `-c one two`, `-c 5`, `-c -- extra`). A `-d` with
//! remaining words still runs its arm against the cleared list, so
//! `-c -d 1 extra` fails with `history position out of range`, rc 1.
//!
//! The plain-listing no_args path is untouched: `history 1 extra` still
//! reports `too many arguments` and abandons the rest of the line
//! (rubash#1660, builtins/common.c no_args()).

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let tag = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    );
    let path = std::env::temp_dir().join(format!("rubash-2637-{tag}.sh"));
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn rubash_c(script: &str) -> (String, String, Option<i32>) {
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
fn history_c_extra_operand_clears_silently() {
    // The issue's regression seed, verbatim.
    let (stdout, stderr, code) = rubash_c(
        "history -s one; history -c extra 2>&1; echo \"rc=$?\"; history",
    );
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn history_c_two_surplus_operands_clears_silently() {
    let (stdout, stderr, code) = rubash_c(
        "history -s one; history -c one two 2>&1; echo \"rc=$?\"; history",
    );
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn history_c_numeric_operand_clears_silently() {
    let (stdout, stderr, code) =
        rubash_c("history -s one; history -c 5 2>&1; echo \"rc=$?\"; history");
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn history_c_dashdash_extra_clears_silently() {
    let (stdout, stderr, code) = rubash_c(
        "history -s one; history -c -- extra 2>&1; echo \"rc=$?\"; history",
    );
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn history_c_delete_consumed_operand_never_runs() {
    // getopt consumes `-d 1`, so no words remain: GNU returns success
    // right after the clear and the delete never runs.
    let (stdout, stderr, code) = rubash_c(
        "history -s one; history -c -d 1 2>&1; echo \"rc=$?\"; history",
    );
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn history_c_delete_bad_offset_never_validated() {
    let (stdout, stderr, code) = rubash_c(
        "history -s one; history -c -d bad 2>&1; echo \"rc=$?\"; history",
    );
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn history_c_save_after_clear_adds_again() {
    // CFLAG clears, then the -s arm still runs: the list ends with
    // just the pushed entry.
    let (stdout, stderr, code) = rubash_c(
        "history -s one; history -c -s two 2>&1; echo \"rc=$?\"; history",
    );
    assert_eq!(stdout, "rc=0\n    1  two\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn history_c_delete_with_remaining_operand_eranges() {
    // Words remain, so the -d arm runs against the cleared (empty)
    // list: `1` is out of range, rc 1.
    let (stdout, _stderr, code) = rubash_file(
        "history -s one\nhistory -c -d 1 extra 2>&1\necho \"rc=$?\"\nhistory\n",
    );
    assert_eq!(stdout, "history: 1: history position out of range\nrc=1\n");
    assert_eq!(code, Some(0));
}

#[test]
fn history_plain_listing_surplus_still_too_many() {
    // Positive control (rubash#1660): without -c, a surplus operand
    // after the count is still `too many arguments` with DISCARD.
    let (stdout, _stderr, code) =
        rubash_file("history -s one\nhistory 1 extra\necho \"ran=$?\"\n");
    assert_eq!(stdout, "ran=2\n");
    assert_eq!(code, Some(0));
}
