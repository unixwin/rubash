//! `trap -p` with a mix of valid and invalid signal specifications
//! (rubash#2638; GNU bash 5.3.20 oracle-verified 2026-11-09).
//!
//! GNU `display_traps` (builtins/trap.def) walks the operand list one
//! item at a time: a decodable signal is printed, an undecodable one
//! draws `trap: <arg>: invalid signal specification` on the spot and
//! processing CONTINUES with the next operand; the builtin returns 1
//! if any operand was invalid. Rubash collected the valid operands in
//! `normalized_signals` correctly but the `-p`/`-P` caller in
//! src/builtins/trap.rs returned early on `signals.invalid`, so the
//! valid items were never printed at all.
//!
//! The pins drive script FILES (diagnostic prefixes carry the script
//! path; the harness normalizes it to `S`) and assert stdout
//! byte-for-byte, including the in-script `2>&1` interleave order,
//! which is exactly GNU's per-item processing order.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn rubash_bin() -> String {
    env!("CARGO_BIN_EXE_rubash").to_string()
}

/// Run a script file; return (normalized stdout, normalized stderr, code).
fn run_script(tag: &str, body: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("issue2638-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    let path: PathBuf = dir.join("case.sh");
    fs::write(&path, body).expect("write script");
    let output = Command::new(rubash_bin())
        .arg(&path)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("run rubash");
    let normalize = |bytes: &[u8]| {
        String::from_utf8_lossy(bytes)
            .replace(path.to_str().expect("utf8 path"), "S")
            .into_owned()
    };
    (
        normalize(&output.stdout),
        normalize(&output.stderr),
        output.status.code(),
    )
}

#[test]
fn issue2638_valid_then_invalid_prints_valid_item() {
    // The issue's canonical shape.
    let (stdout, stderr, code) = run_script(
        "canon",
        "trap \"x\" EXIT; trap -p EXIT extra 2>&1; echo rc=$?\n",
    );
    assert_eq!(
        stdout,
        "trap -- 'x' EXIT\nS: line 1: trap: extra: invalid signal specification\nrc=1\n"
    );
    assert!(stderr.contains("x: command not found"), "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue2638_invalid_then_valid_keeps_argument_order() {
    // The error comes first because the invalid operand comes first;
    // the valid item is still printed after it.
    let (stdout, _stderr, code) = run_script(
        "inv-first",
        "trap \"x\" EXIT; trap -p BADSIG EXIT 2>&1; echo rc=$?\n",
    );
    assert_eq!(
        stdout,
        "S: line 1: trap: BADSIG: invalid signal specification\ntrap -- 'x' EXIT\nrc=1\n"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue2638_sandwiched_invalid_prints_both_neighbours() {
    let (stdout, _stderr, code) = run_script(
        "sandwich",
        "trap \"a\" INT; trap \"b\" EXIT; trap -p INT BADSIG EXIT 2>&1; echo rc=$?\n",
    );
    assert_eq!(
        stdout,
        "trap -- 'a' SIGINT\nS: line 1: trap: BADSIG: invalid signal specification\ntrap -- 'b' EXIT\nrc=1\n"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue2638_print_actions_face_is_per_item_too() {
    // -P shares GNU's display_traps loop (DISP_ACTIONONLY): the valid
    // item prints its action text, the invalid one errors, rc=1, and
    // the EXIT action still runs at exit.
    let (stdout, _stderr, code) = run_script(
        "pface",
        "trap \"echo hi\" EXIT; trap -P EXIT BADSIG 2>&1; echo rc=$?\n",
    );
    assert_eq!(
        stdout,
        "echo hi\nS: line 1: trap: BADSIG: invalid signal specification\nrc=1\nhi\n"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue2638_numeric_signal_spec_is_per_item_too() {
    let (stdout, _stderr, code) = run_script(
        "numeric",
        "trap \"x\" EXIT; trap -p 0 BADSIG 2>&1; echo rc=$?\n",
    );
    assert_eq!(
        stdout,
        "trap -- 'x' EXIT\nS: line 1: trap: BADSIG: invalid signal specification\nrc=1\n"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue2638_all_invalid_reports_each_and_prints_nothing() {
    // Regression lock (both shells agree): every invalid operand gets
    // its own diagnostic, nothing is printed, rc=1.
    let (stdout, _stderr, code) =
        run_script("all-invalid", "trap -p BADSIG1 BADSIG2 2>&1; echo rc=$?\n");
    assert_eq!(
        stdout,
        "S: line 1: trap: BADSIG1: invalid signal specification\nS: line 1: trap: BADSIG2: invalid signal specification\nrc=1\n"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue2638_pure_valid_list_unchanged() {
    // Regression lock: the all-valid listing is untouched by the fix.
    let (stdout, _stderr, code) = run_script(
        "pure-valid",
        "trap \"a\" INT; trap \"b\" EXIT; trap -p INT EXIT 2>&1; echo rc=$?\n",
    );
    assert_eq!(stdout, "trap -- 'a' SIGINT\ntrap -- 'b' EXIT\nrc=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue2638_mixed_listing_does_not_disturb_the_trap_table() {
    // Chain: a mixed -p is a pure query — the same listing afterwards
    // must show the identical table, and a second mixed query repeats
    // the first one's output exactly.
    let (stdout, _stderr, code) = run_script(
        "chain",
        "trap \"x\" EXIT\ntrap -p EXIT BADSIG 2>&1; echo first_rc=$?\ntrap -p EXIT 2>&1; echo second_rc=$?\n",
    );
    assert_eq!(
        stdout,
        "trap -- 'x' EXIT\nS: line 2: trap: BADSIG: invalid signal specification\nfirst_rc=1\ntrap -- 'x' EXIT\nsecond_rc=0\n"
    );
    assert_eq!(code, Some(0));
}
