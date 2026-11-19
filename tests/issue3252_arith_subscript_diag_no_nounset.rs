//! rubash#3252 — without `set -u`, the arithmetic subscript diagnostics
//! for an empty-expanded subscript are entirely absent: `(( b[$undef]++ ))`
//! must report `b[]: bad array subscript` (twice, GNU expr.c:1183/1224 —
//! array_variable_part and get_array_value each diagnose the empty
//! subscript) plus `` ((: `b[]': not a valid identifier `` (GNU
//! expr.c:331-354 expr_bind_variable -> sh_invalidid), as GNU bash 5.3
//! does. On the defective tree the `$undef` expands away to empty and the
//! evaluation silently reads/binds element 0, so stderr is empty.
//!
//! GNU anchors (bash 5.3, oracle binary probed 2026-11-18):
//! - arrayfunc.c:1405-1416 array_variable_name: `ni <= ind + 1` (empty
//!   bracket text after expansion) -> err_badarraysub(s); error.c:440-443
//!   err_badarraysub prints `%s: bad array subscript` with the token text
//!   (`b[]`).
//! - expr.c:1183 expr_streval calls array_variable_part, expr.c:1227 calls
//!   get_array_value — each independently validates the subscript, hence
//!   the doubled line on reads. A plain `=` assignment never reads
//!   (expr.c:1395-1401 peektok == EQ skips expr_streval), so it reports
//!   only the invalid identifier.
//! - expr.c:1044-1054/1101 exp0 post-inc/dec and expr.c:611-614 expassign
//!   with curlval.ind == -1 fall through to expr_bind_variable on the
//!   whole `b[]` token, which fails via sh_invalidid (variables.c).
//! - The discriminator is the ENTRY, not the expansion: `(( ))`, `$(( ))`
//!   and `[[ ]]` run expand_arith_string before evalexp, so the token
//!   reaching evalexp is literally `b[]`; `let` operands arrive
//!   EXP_EXPANDED, so the token is `b[$undef]` and the empty expansion
//!   inside array_expand_index is index 0 — `let 'b[$undef]++'` binds
//!   b[0] silently. A whitespace-only expansion (`b[ $undef ]`) is index
//!   0 in every entry (the bracket text is non-empty).
//!
//! Harness: `rubash -c` via `CARGO_BIN_EXE_rubash`, `LC_ALL=C.UTF-8`,
//! following the adjacent #3199 pin. stderr is matched by substring and
//! by occurrence count, so the shell's error prefix stays free.
//!
//! Evidence for the split-out control: fix-3199-3200 REPORT §15 item 3
//! (box fail-first: `(( b[$undef]++ ))` without -u measured red on stderr
//! only, pre-registered green).

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

fn count(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

/// GNU: `b[]: bad array subscript` exactly `bad` times, `` `b[]': not a
/// valid identifier `` exactly `invalid` times, the given stdout, and the
/// given script exit code.
fn expect_diag(
    script: &str,
    bad: usize,
    invalid: usize,
    expected_stdout: &[u8],
    expected_code: i32,
    ctx: &str,
) {
    let out = run(script);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.stdout, expected_stdout,
        "{ctx}: stdout mismatch; stderr: {stderr}"
    );
    assert_eq!(
        out.code, expected_code,
        "{ctx}: script must exit {expected_code}; stderr: {stderr}"
    );
    assert_eq!(
        count(&stderr, "b[]: bad array subscript"),
        bad,
        "{ctx}: stderr must contain \"b[]: bad array subscript\" {bad} time(s); got {stderr}"
    );
    assert_eq!(
        count(&stderr, "`b[]': not a valid identifier"),
        invalid,
        "{ctx}: stderr must contain \"`b[]': not a valid identifier\" {invalid} time(s); got {stderr}"
    );
}

/// GNU: no diagnostics at all — stderr empty — with the given stdout and
/// script exit code (the silent controls).
fn expect_silent(script: &str, expected_stdout: &[u8], expected_code: i32, ctx: &str) {
    let out = run(script);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.stdout, expected_stdout,
        "{ctx}: stdout mismatch; stderr: {stderr}"
    );
    assert_eq!(
        out.code, expected_code,
        "{ctx}: script must exit {expected_code}; stderr: {stderr}"
    );
    assert_eq!(out.stderr, b"", "{ctx}: stderr must be empty; got {stderr}");
}

#[test]
fn arith_command_postinc_empty_subscript() {
    expect_diag(
        "b=(x y); (( b[$undef]++ )); echo rc=$?; declare -p b",
        2,
        1,
        b"rc=1\ndeclare -a b=([0]=\"x\" [1]=\"y\")\n",
        0,
        "(( b[$undef]++ ))",
    );
}

#[test]
fn arith_command_read_empty_subscript() {
    expect_diag(
        "b=(x y); (( b[$undef] )); echo rc=$?",
        2,
        0,
        b"rc=1\n",
        0,
        "(( b[$undef] ))",
    );
}

#[test]
fn arith_expansion_empty_subscript() {
    expect_diag(
        "b=(x y); echo $(( b[$undef] )); echo rc=$?",
        2,
        0,
        b"0\nrc=0\n",
        0,
        "$(( b[$undef] ))",
    );
}

#[test]
fn arith_command_assign_empty_subscript() {
    // Plain `=` never reads (expr.c:1395-1401): only the invalid
    // identifier, no bad-subscript line, and the bind stores nothing.
    expect_diag(
        "b=(x y); (( b[$undef]=5 )); echo rc=$?; declare -p b",
        0,
        1,
        b"rc=0\ndeclare -a b=([0]=\"x\" [1]=\"y\")\n",
        0,
        "(( b[$undef]=5 ))",
    );
}

#[test]
fn arith_command_plus_assign_empty_subscript() {
    expect_diag(
        "b=(x y); (( b[$undef]+=5 )); echo rc=$?; declare -p b",
        2,
        1,
        b"rc=0\ndeclare -a b=([0]=\"x\" [1]=\"y\")\n",
        0,
        "(( b[$undef]+=5 ))",
    );
}

#[test]
fn arith_command_postdec_empty_subscript() {
    expect_diag(
        "b=(x y); (( b[$undef]-- )); echo rc=$?; declare -p b",
        2,
        1,
        b"rc=1\ndeclare -a b=([0]=\"x\" [1]=\"y\")\n",
        0,
        "(( b[$undef]-- ))",
    );
}

#[test]
fn arith_command_preinc_empty_subscript() {
    // Pre-inc returns the (failed) new value 1, so (( )) exits 0 even
    // though the bind failed (GNU expr.c:1030-1054 exp0).
    expect_diag(
        "b=(x y); (( ++b[$undef] )); echo rc=$?; declare -p b",
        2,
        1,
        b"rc=0\ndeclare -a b=([0]=\"x\" [1]=\"y\")\n",
        0,
        "(( ++b[$undef] ))",
    );
}

#[test]
fn let_double_quoted_empty_subscript() {
    // Double quotes word-expand `$undef` away before `let` sees the
    // operand, so the token reaching evalexp is `b[]++`: same
    // diagnostics as (( )), labelled `let:`.
    let out = run("b=(x y); let \"b[$undef]++\"; echo rc=$?; declare -p b");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.stdout, b"rc=1\ndeclare -a b=([0]=\"x\" [1]=\"y\")\n",
        "let \"b[$undef]++\": stdout mismatch; stderr: {stderr}"
    );
    assert_eq!(
        out.code, 0,
        "let \"b[$undef]++\": script must exit 0; stderr: {stderr}"
    );
    assert_eq!(
        count(&stderr, "b[]: bad array subscript"),
        2,
        "let \"b[$undef]++\": stderr must contain \"b[]: bad array subscript\" twice; got {stderr}"
    );
    assert!(
        stderr.contains("let: `b[]': not a valid identifier"),
        "let \"b[$undef]++\": stderr must contain \"let: `b[]': not a valid identifier\"; got {stderr}"
    );
}

#[test]
fn let_single_quoted_empty_subscript_silent_control() {
    // Single quotes keep `$undef` in the operand: the token reaching
    // evalexp is `b[$undef]` (non-empty bracket text) and the empty
    // expansion inside array_expand_index is index 0 — GNU binds b[0]
    // silently.
    expect_silent(
        "b=(x y); let 'b[$undef]++'; echo rc=$?; declare -p b",
        b"rc=1\ndeclare -a b=([0]=\"1\" [1]=\"y\")\n",
        0,
        "let 'b[$undef]++'",
    );
}

#[test]
fn let_single_quoted_empty_quotes_silent_control() {
    // Same law for a literal empty-quoted subscript under `let`.
    expect_silent(
        "b=(x y); let 'b[\"\"]++'; echo rc=$?; declare -p b",
        b"rc=1\ndeclare -a b=([0]=\"1\" [1]=\"y\")\n",
        0,
        "let 'b[\"\"]++'",
    );
}

#[test]
fn arith_command_spaced_subscript_silent_control() {
    // Whitespace-only bracket text is non-empty: GNU evaluates it to
    // index 0 with no diagnostic (`(( b[ $undef ]++ ))` binds b[0]).
    expect_silent(
        "b=(x y); (( b[ $undef ]++ )); echo rc=$?; declare -p b",
        b"rc=1\ndeclare -a b=([0]=\"1\" [1]=\"y\")\n",
        0,
        "(( b[ $undef ]++ ))",
    );
}

#[test]
fn arith_command_double_quoted_empty_subscript() {
    // `""` quote-removes to empty in the (( )) expansion pass, so the
    // token is `b[]`: same diagnostics as the `$undef` form.
    expect_diag(
        "b=(x y); (( b[\"\"]++ )); echo rc=$?; declare -p b",
        2,
        1,
        b"rc=1\ndeclare -a b=([0]=\"x\" [1]=\"y\")\n",
        0,
        "(( b[\"\"]++ ))",
    );
}

#[test]
fn arith_command_unset_array_empty_subscript() {
    // The array itself being unset changes nothing: the token parse
    // fails before any cell is looked up.
    expect_diag(
        "(( b[$undef]++ )); echo rc=$?",
        2,
        1,
        b"rc=1\n",
        0,
        "(( b[$undef]++ )) with b unset",
    );
}

#[test]
fn arith_expansion_assign_empty_subscript_unlabeled() {
    // Inside `$(( ))` this_command_name is NULL (subst.c:10873-10878
    // arithsub window), so the invalid-identifier line carries no
    // `((:` label; the assignment still yields its RHS value 5.
    let out = run("b=(x y); echo $(( b[$undef]=5 )); echo rc=$?; declare -p b");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.stdout, b"5\nrc=0\ndeclare -a b=([0]=\"x\" [1]=\"y\")\n",
        "$(( b[$undef]=5 )): stdout mismatch; stderr: {stderr}"
    );
    assert_eq!(
        out.code, 0,
        "$(( b[$undef]=5 )): script must exit 0; stderr: {stderr}"
    );
    assert_eq!(
        count(&stderr, "b[]: bad array subscript"),
        0,
        "$(( b[$undef]=5 )): no bad-subscript line; got {stderr}"
    );
    assert!(
        stderr.contains("`b[]': not a valid identifier"),
        "$(( b[$undef]=5 )): stderr must contain \"`b[]': not a valid identifier\"; got {stderr}"
    );
    assert!(
        !stderr.contains("((:"),
        "$(( b[$undef]=5 )): the invalid-identifier line must carry no ((: label; got {stderr}"
    );
}

#[test]
fn conditional_empty_subscript() {
    // `[[ ]]` word-expands the operand (cond_expand_word) before
    // arithcomp's evalexp, so the read diagnoses twice; the value 0
    // makes `-eq 0` true.
    expect_diag(
        "b=(x y); [[ b[$undef] -eq 0 ]]; echo rc=$?",
        2,
        0,
        b"rc=0\n",
        0,
        "[[ b[$undef] -eq 0 ]]",
    );
}

#[test]
fn bound_subscript_numeric_element_control() {
    // Control: a bound `$undef` over a numeric element still evaluates
    // (GNU: b[0] 5 -> 6, (( )) sees the old value 5, rc 0).
    expect_silent(
        "b=(5 y); undef=0; (( b[$undef]++ )); echo rc=$?; echo ${b[0]}",
        b"rc=0\n6\n",
        0,
        "bound $undef over numeric element",
    );
}

#[test]
fn value_recursion_empty_subscript_silent_control() {
    // A variable value re-enters subexpr verbatim (expr.c:1241): the
    // bracket text `b[$undef]` is non-empty, so its empty expansion
    // inside array_expand_index is index 0, silently.
    expect_silent(
        "x='b[$undef]'; echo $(( x )); echo rc=$?",
        b"0\nrc=0\n",
        0,
        "$(( x )) with x='b[$undef]'",
    );
}

#[test]
fn assoc_command_postinc_empty_subscript_lock() {
    // Associative face, regression lock (rubash#860 empty-span
    // machinery already routes the empty resolution to the `name[]`
    // law): `C[]: bad array subscript` twice plus the invalid
    // identifier, rc 1.
    let out = run("declare -A C; (( C[$undef]++ )); echo rc=$?");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.stdout, b"rc=1\n",
        "(( C[$undef]++ )): stdout mismatch; stderr: {stderr}"
    );
    assert_eq!(
        out.code, 0,
        "(( C[$undef]++ )): script must exit 0; stderr: {stderr}"
    );
    assert_eq!(
        count(&stderr, "C[]: bad array subscript"),
        2,
        "(( C[$undef]++ )): stderr must contain \"C[]: bad array subscript\" twice; got {stderr}"
    );
    assert_eq!(
        count(&stderr, "`C[]': not a valid identifier"),
        1,
        "(( C[$undef]++ )): stderr must contain \"`C[]': not a valid identifier\" once; got {stderr}"
    );
}

#[test]
fn assoc_command_read_empty_subscript_lock() {
    let out = run("declare -A C; (( C[$undef] )); echo rc=$?");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.stdout, b"rc=1\n",
        "(( C[$undef] )): stdout mismatch; stderr: {stderr}"
    );
    assert_eq!(
        out.code, 0,
        "(( C[$undef] )): script must exit 0; stderr: {stderr}"
    );
    assert_eq!(
        count(&stderr, "C[]: bad array subscript"),
        2,
        "(( C[$undef] )): stderr must contain \"C[]: bad array subscript\" twice; got {stderr}"
    );
    assert_eq!(
        count(&stderr, "not a valid identifier"),
        0,
        "(( C[$undef] )): no invalid-identifier line; got {stderr}"
    );
}
