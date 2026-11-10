//! rubash#3200 — `set -u` is silent for the recursive evaluation of
//! an array element's value as an arithmetic expression: with
//! `b=(x y)` and `i=1`, `(( b[i]++ ))` must report
//! `y: unbound variable` with rc 127, as GNU does. On the defective
//! tree the element value `y` is re-parsed as an expression whose
//! variable lookup silently reads an unset name as 0, so the command
//! stores `b[1]=1` and passes.
//!
//! GNU anchors (bash 5.3, oracle binary probed 2026-11-10):
//! - expr.c:1190-1216 expr_streval: an unset (or invisible) variable
//!   reference with `unbound_vars_is_error` set aborts with
//!   err_unboundvar. The element value reaches expr_streval through
//!   array_expand_index (arrayfunc.c:1368-1378) / the `e == ']'` arm
//!   of expr_streval (expr.c:1178-1183), so a non-numeric element
//!   value traps exactly like a bare unset name.
//! - expr.c:1241: the fetched value text is re-evaluated with
//!   `tval = (value && *value) ? subexpr (value) : 0` — the recursion
//!   is a property of value evaluation, shared by the indexed,
//!   associative and scalar read paths, so one check at the value
//!   evaluator covers every entry form (`(( ))`, `$(( ))`, `let`).
//! - In a non-interactive shell the error aborts the script
//!   (jump_to_top_level FORCE_EOF) with exit status 127.
//!
//! Correction to the filed shape: with `i` UNSET, GNU reports
//! `i: unbound variable` (the subscript index itself is the unbound
//! read), not `y` — `y` is reported only once the index resolves
//! (`i=1`, the A1 residual-2 form). Both shapes are pinned here.
//!
//! Harness: `rubash -c` via `CARGO_BIN_EXE_rubash`, `LC_ALL=C.UTF-8`,
//! following the adjacent #3132 pin (issue2654).

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
fn arith_command_element_value_unbound() {
    // Headline (A1 residual 2): index resolves, element value `y` is
    // re-evaluated as an expression, `y` is unbound.
    expect_unbound(
        "set -u; b=(x y); i=1; (( b[i]++ )); echo AFTER",
        "y",
        "(( b[i]++ )) with i=1",
    );
}

#[test]
fn arith_command_unset_index_unbound() {
    // Filed shape corrected per oracle: with `i` unset, GNU reports
    // the index read itself (`i`), not the element value.
    expect_unbound(
        "set -u; b=(x y); (( b[i]++ )); echo AFTER",
        "i",
        "(( b[i]++ )) with i unset",
    );
}

#[test]
fn arith_expansion_element_value_unbound() {
    expect_unbound(
        "set -u; b=(x y); i=1; echo $(( b[i] )); echo AFTER",
        "y",
        "$(( b[i] )) with i=1",
    );
}

#[test]
fn bound_dollar_subscript_y_element_unbound() {
    // Cross point with #3199: the subscript resolves through a bound
    // `$undef`, then the element value `y` is re-evaluated — needs both
    // fixes to report `y`.
    expect_unbound(
        "set -u; b=(5 y); undef=1; (( b[$undef]++ )); echo AFTER",
        "y",
        "(( b[$undef]++ )) with undef=1 over y",
    );
}

#[test]
fn bound_subscript_x_element_unbound() {
    // Same cross point via #3199's form: b[0] holds `x`, re-evaluated.
    expect_unbound(
        "set -u; b=(x y); undef=0; (( b[$undef]++ )); echo AFTER",
        "x",
        "(( b[$undef]++ )) with undef=0 over x",
    );
}

#[test]
fn literal_subscript_element_value_unbound() {
    // Supplementary cell (residual-lane oracle probe C3): a literal
    // index reaches the same value recursion — b[0] holds `x`.
    expect_unbound(
        "set -u; b=(x y); (( b[0]++ )); echo AFTER",
        "x",
        "(( b[0]++ )) literal index",
    );
}

#[test]
fn assoc_element_value_unbound() {
    // Supplementary cell (residual-lane oracle probe R2j):
    // associative elements take the same value-evaluation path.
    expect_unbound(
        "set -u; declare -A A=([k]=y); (( A[k]++ )); echo AFTER",
        "y",
        "(( A[k]++ )) assoc element",
    );
}

#[test]
fn let_element_value_unbound() {
    // Supplementary cell (residual-lane oracle probe R2h): the `let`
    // entry form shares the value evaluator with `(( ))`.
    expect_unbound(
        "set -u; b=(x y); i=1; let 'b[i]++'; echo AFTER",
        "y",
        "let 'b[i]++' with i=1",
    );
}

#[test]
fn numeric_element_evaluates_under_nounset() {
    // Control: a numeric element value still evaluates under `set -u`
    // (GNU: b[1] 5 -> 6, (( )) sees the old value 5, rc 0).
    expect_stdout(
        "set -u; b=(x 5); i=1; (( b[i]++ )); echo rc=$?; echo ${b[1]}",
        b"rc=0\n6\n",
        "numeric element under set -u",
    );
}

#[test]
fn numeric_element_read_under_nounset() {
    expect_stdout(
        "set -u; b=(x 5); i=1; echo $(( b[i] )); echo rc=$?",
        b"5\nrc=0\n",
        "$(( b[i] )) numeric under set -u",
    );
}

#[test]
fn plain_var_unbound_still_traps() {
    // Control: a bare unset name already traps; the #3200 fix must not
    // regress it.
    expect_unbound("set -u; (( z )); echo AFTER", "z", "(( z ))");
}
