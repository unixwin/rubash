//! rubash#3563 — arithmetic `set -u` residuals left by #3561 (#3496/#3497/#3498).
//!
//! GNU anchors (bash 5.3.20, vendored tree @9c465866):
//! - Row 1: `[[ ]]` arithcomp operands are evaluated by evalexp, whose
//!   `noeval` short-circuit never reads the right side of `0 && u`
//!   (expr.c:1164-1167, expr.c:1190-1216 expr_streval). A text scan over
//!   the expanded operand must not trap on a name evalexp never reads.
//! - Rows 2-3: the word walk expands left-to-right (subst.c:11229
//!   expand_word_internal), so a `$((k=1))` / `$((h[k]=4))` binds BEFORE
//!   a later word's `$k` / `${h[k]}` is judged.
//! - Row 4: an arithmetic subscript is expanded in an unquoted context
//!   (subst.c:11147), where a single-quoted span is literal data —
//!   `${undef:-'$q'}` hands evalexp the literal `$q`, an operand error,
//!   never a nounset read of `q`.
//! - Row 5: expr.c:1194 `array_variable_name (tok)` reports the
//!   reference name (`r`), not the nameref target (`zz`).
//! - Row 6: a `declare -i` assignment evaluates its RHS through evalexp
//!   (variables.c make_variable_value), so an unbound read traps
//!   `undef: unbound variable` instead of a synthesized syntax error.
//! - Row 7: an array-element assignment's subscript nounset trap is
//!   FORCE_EOF (expr.c:1190-1216 err_unboundvar), not a status-1
//!   continue.
//!
//! Every expectation below is the GNU bash 5.3.20 `bash -c` measurement,
//! byte-for-byte on the message tail.

use std::process::{Command, Stdio};

struct Outcome {
    stdout: String,
    stderr_tail: String,
    code: i32,
}

/// Run `rubash -c script`; the stderr lines lose their `<argv0>: line N: `
/// prefix (it names the binary, which differs per checkout).
fn run(script: &str) -> Outcome {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr_tail = stderr
        .lines()
        .map(|line| match line.find(": line 1: ") {
            Some(at) => &line[at + ": line 1: ".len()..],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n");
    Outcome {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr_tail,
        code: output.status.code().unwrap_or(-1),
    }
}

fn expect(script: &str, stdout: &str, stderr_tail: &str, code: i32) {
    let out = run(script);
    assert_eq!(out.stdout, stdout, "stdout of {script:?}");
    assert_eq!(out.stderr_tail, stderr_tail, "stderr of {script:?}");
    assert_eq!(out.code, code, "exit status of {script:?}");
}

fn expect_unbound(script: &str, name: &str) {
    expect(script, "", &format!("{name}: unbound variable"), 127);
}

// ---- Row 1: [[ ]] operands short-circuit in evalexp ----

#[test]
fn cond_arith_noeval_operand_never_traps() {
    expect("set -u; [[ 0 -eq '0 && u' ]]; echo rc=$?", "rc=0\n", "", 0);
    expect("set -u; [[ 1 -eq '0 && u' ]]; echo rc=$?", "rc=1\n", "", 0);
}

#[test]
fn cond_arith_real_reads_still_trap() {
    expect_unbound("set -u; [[ u -eq 0 ]]; echo rc=$?", "u");
    expect_unbound("set -u; [[ 0 -eq u ]]; echo rc=$?", "u");
}

// ---- Rows 2-3: an earlier $((...)) binds before later words ----

#[test]
fn arith_assignment_binds_for_later_words() {
    expect("set -u; echo $(( k=1 )) $k", "1 1\n", "", 0);
    expect("set -u; echo $(( j=2 )) $(( j+1 ))", "2 3\n", "", 0);
    expect(
        "set -u; declare -A h; echo $(( h[k]=4 )) ${h[k]}",
        "4 4\n",
        "",
        0,
    );
}

#[test]
fn arith_compound_assignment_still_reads_first() {
    expect_unbound("set -u; echo $(( k+=1 )) $k", "k");
}

// ---- Row 4: single-quoted span inside a ${...:-...} subscript word ----

#[test]
fn subscript_default_single_quoted_dollar_is_data() {
    // GNU 5.3.20 (both `-c` and script-file form): the subscript text
    // `$q` reaches evalexp and its operand error is FATAL to the
    // non-interactive shell — `echo rc=$?` never runs, exit status 1.
    expect(
        "set -u; b=(x y); (( b[${undef:-'$q'}] )); echo rc=$?",
        "",
        "$q: arithmetic syntax error: operand expected (error token is \"$q\")",
        1,
    );
}

// ---- Row 5: nameref diagnostic names the reference ----

#[test]
fn nameref_unbound_reports_reference_name() {
    // Array reference: expr.c:1194 array_variable_name (tok) names the
    // reference. A bare scalar read follows the nameref and names the
    // target instead (verified GNU 5.3.20: `zz: unbound variable`).
    expect_unbound("set -u; declare -n r=zz; echo $(( r[0] )); echo rc=$?", "r");
    expect_unbound("set -u; declare -n r=zz; echo $(( r )); echo rc=$?", "zz");
}

#[test]
fn nameref_bound_read_still_works() {
    expect(
        "set -u; zz=5; declare -n r=zz; echo $(( r[0] ))",
        "5\n",
        "",
        0,
    );
    expect(
        "set -u; declare -n r=zz; zz=7; echo $(( r )); echo $(( r+1 ))",
        "7\n8\n",
        "",
        0,
    );
}

// ---- Row 6: declare -i assignment RHS nounset ----

#[test]
fn integer_assignment_rhs_unbound_traps() {
    expect_unbound("set -u; declare -i di; di=undef+1; echo rc=$?", "undef");
}

#[test]
fn integer_assignment_rhs_still_evaluates() {
    expect("set -u; declare -i di; di=1+2; echo $di", "3\n", "", 0);
}

// ---- Row 7: array-element assignment subscript nounset aborts ----

#[test]
fn array_element_subscript_unbound_aborts() {
    expect_unbound("set -u; a[undef]=1; echo ${a[0]}; echo AFTER", "undef");
}

#[test]
fn array_element_subscript_bound_still_binds() {
    expect("set -u; i=0; a[i]=1; echo ${a[0]}", "1\n", "", 0);
}
