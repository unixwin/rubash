//! `shift` count edge regressions (GNU bash 5.3 baseline, byte-verified
//! against local bash 5.3.20 2026-10-20; WSL 5.3.0 re-check by the main
//! session).
//!
//! Ownership: `builtins/shift.def` (`get_numeric_arg` + `sh_neednumarg` in
//! `builtins/common.c`, `no_args` surplus-operand path). The pins:
//!
//! - out-of-range (past the end, or negative) is rc 1 and MUST print, and
//!   leaves `$#`/`$@` untouched;
//! - non-numeric operands are rc 2 (`EX_USAGE`) and also leave the
//!   positional array untouched; surplus operands take the `no_args`
//!   DISCARD path instead (see S4) — the rest of the list is
//!   abandoned, so no following command observes a status at all;
//! - `shift --` strips the terminator first, bare `shift --` shifts by 1;
//! - a successful shift rewrites `$#`/`$@`, and function frames are
//!   isolated from the caller's positionals.

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// S1: shifting past the end is rc 1 and keeps the positional array.
#[test]
fn s1_shift_past_end_keeps_positionals() {
    let (stdout, stderr, code) = rubash("set -- a b c; shift 5; echo \"rc=$? n=$# args=$*\"");
    assert_eq!(stderr, "", "over-shift must stay silent: {stderr:?}");
    assert_eq!(stdout, "rc=1 n=3 args=a b c\n");
    assert_eq!(code, Some(0));
}

/// S2: `shift == $#` empties (rc 0); empty-stack `shift` is rc 1;
/// `shift 0` is a no-op rc 0.
#[test]
fn s2_exact_empty_and_zero_counts() {
    let (stdout, stderr, _) = rubash("set -- a b c; shift 3; echo \"rc=$? n=$#\"");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "rc=0 n=0\n");

    let (stdout, stderr, _) = rubash("set --; shift; echo \"rc=$? n=$#\"");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "rc=1 n=0\n");

    let (stdout, stderr, _) = rubash("set -- a b c; shift 0; echo \"rc=$? n=$# args=$*\"");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "rc=0 n=3 args=a b c\n");
}

/// S3: a negative count is rc 1 and MUST print `shift count out of range`.
#[test]
fn s3_negative_count_prints_and_fails() {
    let (stdout, stderr, _) = rubash("set -- a b; shift -1; echo \"rc=$? n=$# args=$*\"");
    assert!(
        stderr.contains("shift count out of range"),
        "negative shift must diagnose: {stderr:?}"
    );
    assert_eq!(stdout, "rc=1 n=2 args=a b\n");
}

/// S4: non-numeric operands (including empty and huge) are rc 2 with
/// `numeric argument required`.
#[test]
fn s4_non_numeric_count_is_usage_error() {
    for operand in ["x", "", "999999999999999999999"] {
        let (stdout, stderr, _) = rubash(&format!(
            "set -- a b; shift '{operand}'; echo \"rc=$? n=$# args=$*\""
        ));
        assert!(
            stderr.contains("numeric argument required"),
            "operand {operand:?} must diagnose: {stderr:?}"
        );
        assert_eq!(stdout, "rc=2 n=2 args=a b\n", "operand {operand:?}");
    }
    // Surplus operands take a different path from the non-numeric
    // ones above: get_numeric_arg (builtins/common.c:488) ends in
    // no_args (common.c:511), and no_args (common.c:139-151)
    // reports `too many arguments` and does
    // jump_to_top_level(DISCARD) — the rest of the command list is
    // abandoned, so the echo never runs. Oracle-verified
    // (READJUDICATION, adj-semantics §shift s4): script file ->
    // no stdout, exit 2; -c (this harness) -> no stdout, exit 1.
    // The original pin expected the echo to run with rc=2.
    let (stdout, stderr, code) = rubash("set -- a b; shift 1 2; echo \"rc=$? n=$#\"");
    assert!(stderr.contains("too many arguments"), "{stderr:?}");
    assert_eq!(stdout, "", "the DISCARD must abandon the echo: {stdout:?}");
    assert_eq!(code, Some(1));
}

/// S5: leading `--` is skipped; bare `shift --` shifts by exactly 1.
#[test]
fn s5_double_dash_terminator() {
    let (stdout, stderr, _) = rubash("set -- x y z; shift -- 2; echo \"rc=$? n=$# args=$*\"");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "rc=0 n=1 args=z\n");

    let (stdout, stderr, _) = rubash("set -- x y z; shift --; echo \"rc=$? n=$# args=$*\"");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "rc=0 n=2 args=y z\n");
}

/// S6: `$#`/`$@` are rewritten on success and frozen on failure.
#[test]
fn s6_hash_and_at_follow_shift_outcome() {
    let (stdout, stderr, _) = rubash("set -- a b c; shift 2; echo \"rc=$? n=$# at=$@\"");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "rc=0 n=1 at=c\n");

    let (stdout, stderr, _) = rubash("set -- a b c; shift 9; echo \"rc=$? n=$# at=$@\"");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "rc=1 n=3 at=a b c\n");
}

/// S7: shifting inside a function frame never touches the caller's array.
#[test]
fn s7_function_frame_is_isolated() {
    let (stdout, stderr, _) = rubash(
        "f() { shift 2; echo \"in-f n=$# args=$*\"; }; \
         set -- 1 2 3 4; f a b c; echo \"outer n=$# args=$*\"",
    );
    assert_eq!(stderr, "");
    assert_eq!(stdout, "in-f n=1 args=c\nouter n=4 args=1 2 3 4\n");
}
