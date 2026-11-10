//! rubash#3199 — `set -u` is silent for a `$`-expansion inside an
//! arithmetic array subscript: `(( b[$undef]++ ))` (and the read form
//! `(( b[$undef] ))`) must report `undef: unbound variable` with
//! rc 127, as GNU does. On the defective tree the `$undef` is expanded
//! away to empty before the nounset scan ever runs, so the command
//! falls through to a different error shape (`b[]: bad array
//! subscript`) or, on some paths, passes silently.
//!
//! GNU anchors (bash 5.3, oracle binary probed 2026-11-10):
//! - subst.c:10464 param_expand: a `$name` inside the subscript is
//!   expanded before the array itself is evaluated; its
//!   `unbound_variable:` label (subst.c:11024-11037) reports the name
//!   after the `$` via err_unboundvar. The subscript is its own frame
//!   (arrayfunc.c:1368-1378 array_expand_index).
//! - subst.c:11107 expand_array_subscript (reached from
//!   expand_word_internal's `[` arm, subst.c:11395-11405): the
//!   subscript is expanded as a unit, and a `$name` inside it goes
//!   through parameter expansion, which traps an unbound read when
//!   `unbound_vars_is_error` is set (subst.c:7500).
//! - expr.c:1190-1216 expr_streval: the same trap at evaluation time;
//!   in a non-interactive shell the error aborts the script
//!   (jump_to_top_level FORCE_EOF) with exit status 127, so no
//!   trailing command runs.
//!
//! Harness: `rubash -c` via `CARGO_BIN_EXE_rubash`, `LC_ALL=C.UTF-8`,
//! following the adjacent #3132 pin (issue2654). stderr is matched by
//! substring (`{name}: unbound variable`) so the shell's error prefix
//! stays free.
//!
//! Re-scope note (captain ruling 2026-11-10 on the measured base
//! fail-first, REPORT §15/§16): two control cells measured RED on the
//! base for reasons outside this issue's fix surface were moved out
//! of this pin — the bare `(( $undef ))` control belongs to
//! rubash#3251 and the no-`set -u` subscript-diagnostic control to
//! rubash#3252. Their evidence is not deleted: it stays in REPORT
//! §15 with the box probe logs (`ff.log` / `probe2.log`) it cites,
//! and their pins land with those fix waves. Base fail-first for
//! the six cells below is 2 passed / 4 failed — the
//! `$(( b[$undef] ))` cell and the bound-numeric control were
//! already green (regression locks); all six must pass on the
//! fixed tree.

use std::process::{Command, Stdio};

struct Outcome {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    code: i32,
}

fn run(script: &str) -> Outcome {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    Outcome {
        stdout: output.stdout,
        stderr: output.stderr,
        code: output.status.code().unwrap_or(-1),
    }
}

/// GNU: the nounset error aborts the non-interactive script — no
/// stdout at all, exit 127, and `{name}: unbound variable` on stderr.
fn expect_unbound(script: &str, name: &str, ctx: &str) {
    let out = run(script);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let needle = format!("{name}: unbound variable");
    assert_eq!(
        out.stdout,
        b"",
        "{ctx}: GNU aborts the script on the nounset error, so there must be no stdout; stderr: {stderr}"
    );
    assert_eq!(
        out.code, 127,
        "{ctx}: script must exit 127; stderr: {stderr}"
    );
    assert!(
        stderr.contains(&needle),
        "{ctx}: stderr must contain {needle:?}; got {stderr}"
    );
}

fn expect_stdout(script: &str, expected_stdout: &[u8], ctx: &str) {
    let out = run(script);
    assert_eq!(
        out.stdout,
        expected_stdout,
        "{ctx}: stdout mismatch; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        out.code,
        0,
        "{ctx}: script must exit 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn arith_command_subscript_dollar_unbound() {
    expect_unbound(
        "set -u; b=(x y); (( b[$undef]++ )); echo AFTER",
        "undef",
        "(( b[$undef]++ ))",
    );
}

#[test]
fn arith_command_subscript_dollar_unbound_array_itself_unset() {
    // Must-check cell (captain ruling, R1 residual shape; GNU oracle
    // probe R1a): the array `b` is itself unset. GNU still reports the
    // subscript's `$undef` — subscript expansion (subst.c:10464
    // param_expand) runs before the evaluator ever looks at the array
    // name, so `b: unbound variable` must NOT appear. On the #3132
    // tree this shape exits 127 but names `b`: the pre-expansion
    // trace has to be consulted before the nounset scan reports the
    // array name, or this cell stays red.
    let script = "set -u; (( b[$undef]++ )); echo AFTER";
    let out = run(script);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.stdout, b"",
        "array-unset R1: GNU aborts the script, no stdout; stderr: {stderr}"
    );
    assert_eq!(
        out.code, 127,
        "array-unset R1: script must exit 127; stderr: {stderr}"
    );
    assert!(
        stderr.contains("undef: unbound variable"),
        "array-unset R1: stderr must contain \"undef: unbound variable\"; got {stderr}"
    );
    assert!(
        !stderr.contains("b: unbound variable"),
        "array-unset R1: the array name must not be reported before the subscript expansion; got {stderr}"
    );
}

#[test]
fn arith_command_subscript_dollar_read_unbound() {
    expect_unbound(
        "set -u; b=(x y); (( b[$undef] )); echo AFTER",
        "undef",
        "(( b[$undef] ))",
    );
}

#[test]
fn arith_expansion_subscript_dollar_unbound() {
    expect_unbound(
        "set -u; b=(x y); echo $(( b[$undef] )); echo AFTER",
        "undef",
        "$(( b[$undef] ))",
    );
}

#[test]
fn let_subscript_dollar_unbound() {
    expect_unbound(
        "set -u; b=(x y); let 'b[$undef]++'; echo AFTER",
        "undef",
        "let 'b[$undef]++'",
    );
}

#[test]
fn bound_subscript_numeric_element_evaluates() {
    // Control: a bound `$undef` over a numeric element still evaluates
    // (GNU: b[0] 5 -> 6, (( )) sees the old value 5, rc 0).
    expect_stdout(
        "set -u; b=(5 y); undef=0; (( b[$undef]++ )); echo rc=$?; echo ${b[0]}",
        b"rc=0\n6\n",
        "bound $undef over numeric element",
    );
}
