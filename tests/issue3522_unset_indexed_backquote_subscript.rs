//! Issue rubash#3522 (#3493 residual 2): `unset` of an indexed-array
//! element whose subscript reaches the evaluator through a parameter
//! expansion — `x='`echo 1`'; unset 'a[${x}]'` — silently removed the
//! element (rc 0) where GNU fails with an arithmetic diagnostic (rc 1,
//! element survives).
//!
//! Root cause: `eval_indexed_subscript_expression` (arrayfunc.c:1353-1391
//! `array_expand_index`) ran the `expand_arith_string` pass
//! (`expand_arithmetic_subscript_mut`) on text `resolve_array_subscript`
//! had ALREADY expanded once. GNU's `expand_arith_string` (subst.c:3981)
//! runs one `expand_word_internal` pass on the subscript text as received:
//! text a parameter expansion produces is data, never re-scanned for
//! command substitution — so `${x}` expands to backquote text and
//! `evalexp` fails it as "operand expected" (expr.c:1120). Rubash's
//! second walker pass executed the produced backquote/`$(...)`/`$name`
//! as fresh syntax and removed the element.
//!
//! Fix: the `expand_arith_string` pass runs only when the text is still
//! raw (Protected/verbatim subscripts from assignments, W_ARRAYREF
//! operands and `test -v`, and the compound-declare paths); Raw /
//! ExpandedOnce subscripts and the `${}`-pattern path evaluate the
//! resolved text as-is.

use std::process::Command;

fn rubash_file(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-issue3522-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create dir");
    let path = dir.join("case.sh");
    std::fs::write(&path, script).expect("write script");
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.arg0("bash");
    }
    let output = command
        .arg(&path)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let rc = output.status.code();
    let _ = std::fs::remove_dir_all(&dir);
    (stdout, stderr, rc)
}

/// Failing unset: exactly one diagnostic line containing `diag`, rc 1,
/// array unchanged.
fn expect_fail(tag: &str, script: &str, diag: &str) {
    let (stdout, stderr, rc) = rubash_file(tag, script);
    assert_eq!(stderr.lines().count(), 1, "stderr lines: {stderr}");
    assert!(stderr.contains(diag), "stderr missing {diag:?}: {stderr}");
    assert_eq!(rc, Some(1), "rc: stdout={stdout} stderr={stderr}");
    assert_eq!(
        stdout, "declare -a a=([0]=\"x\" [1]=\"y\" [2]=\"z\")\n",
        "array changed: {stdout}"
    );
}

/// Successful unset/assignment: no stderr, rc 0, exact stdout.
fn expect_ok(tag: &str, script: &str, stdout_expected: &str) {
    let (stdout, stderr, rc) = rubash_file(tag, script);
    assert_eq!(stderr, "", "stderr not empty: {stderr}");
    assert_eq!(rc, Some(0), "rc: stdout={stdout} stderr={stderr}");
    assert_eq!(stdout, stdout_expected, "wrong stdout");
}

const REMOVED_1: &str = "declare -a a=([0]=\"x\" [2]=\"z\")\n";
const REMOVED_2: &str = "declare -a a=([0]=\"x\" [1]=\"y\")\n";
const BACKQUOTE_1: &str =
    "`echo 1`: arithmetic syntax error: operand expected (error token is \"`echo 1`\")";

/// The issue repro, verbatim: backquote text via `${x}` is data, not syntax.
#[test]
fn unset_backquote_text_via_braced_parameter_fails() {
    expect_fail(
        "backquote-braced",
        "a=(x y z)\nx='`echo 1`'\nunset 'a[${x}]'\nrc=$?\ndeclare -p a\nexit \"$rc\"\n",
        BACKQUOTE_1,
    );
}

/// Backquote text with invalid arithmetic output, via `${x}`: evalexp
/// stops at the backquote itself, not at the produced `1 2`.
#[test]
fn unset_backquote_invalid_output_via_braced_parameter_fails() {
    expect_fail(
        "backquote-invalid-braced",
        "a=(x y z)\nx='`echo \"1 2\"`'\nunset 'a[${x}]'\nrc=$?\ndeclare -p a\nexit \"$rc\"\n",
        r#"`echo "1 2"`: arithmetic syntax error: operand expected (error token is "`echo "1 2"`")"#,
    );
}

/// `$(...)` text via `${x}` is data too.
#[test]
fn unset_command_substitution_via_braced_parameter_fails() {
    expect_fail(
        "comsub-braced",
        "a=(x y z)\nx='$(echo 1)'\nunset 'a[${x}]'\nrc=$?\ndeclare -p a\nexit \"$rc\"\n",
        "$(echo 1): arithmetic syntax error: operand expected (error token is \"$(echo 1)\")",
    );
}

/// `$name` text via `${x}` is data: evalexp never looks `$y` up.
#[test]
fn unset_dollar_name_via_braced_parameter_fails() {
    expect_fail(
        "dollar-name-braced",
        "a=(x y z)\ny=1\nx='$y'\nunset 'a[${x}]'\nrc=$?\ndeclare -p a\nexit \"$rc\"\n",
        "$y: arithmetic syntax error: operand expected (error token is \"$y\")",
    );
}

/// `$((...))` text via `${x}` is data.
#[test]
fn unset_arith_substitution_via_braced_parameter_fails() {
    expect_fail(
        "arith-subst-braced",
        "a=(x y z)\nx='$((1+1))'\nunset 'a[${x}]'\nrc=$?\ndeclare -p a\nexit \"$rc\"\n",
        "$((1+1)): arithmetic syntax error: operand expected (error token is \"$((1+1))\")",
    );
}

/// Same face, unbraced `$x`.
#[test]
fn unset_backquote_text_via_unbraced_parameter_fails() {
    expect_fail(
        "backquote-unbraced",
        "a=(x y z)\nx='`echo 1`'\nunset 'a[$x]'\nrc=$?\ndeclare -p a\nexit \"$rc\"\n",
        BACKQUOTE_1,
    );
}

/// Same face through a nameref: the rebuilt `a[${x}]` operand still
/// goes through the ExpandedOnce path.
#[test]
fn unset_backquote_text_via_nameref_braced_parameter_fails() {
    expect_fail(
        "backquote-nameref",
        "a=(x y z)\ndeclare -n n=a\nx='`echo 1`'\nunset 'n[${x}]'\nrc=$?\ndeclare -p a\nexit \"$rc\"\n",
        BACKQUOTE_1,
    );
}

/// Backquote text inside a compound subscript, via `${x}`.
#[test]
fn unset_backquote_text_in_compound_arithmetic_via_parameter_fails() {
    expect_fail(
        "backquote-compound",
        "a=(x y z)\nx='`echo 1`'\nunset 'a[${x}+1]'\nrc=$?\ndeclare -p a\nexit \"$rc\"\n",
        "`echo 1`+1: arithmetic syntax error: operand expected (error token is \"`echo 1`+1\")",
    );
}

/// Backquote text after `+`, via `${x}`.
#[test]
fn unset_backquote_text_after_plus_via_parameter_fails() {
    expect_fail(
        "backquote-after-plus",
        "a=(x y z)\nx='`echo 1`'\nunset 'a[1+${x}]'\nrc=$?\ndeclare -p a\nexit \"$rc\"\n",
        "1+`echo 1`: arithmetic syntax error: operand expected (error token is \"`echo 1`\")",
    );
}

/// Guard: the assignment path keeps its single pass — a backquote
/// parameter there still fails (Protected text is raw, not re-expanded).
#[test]
fn assign_backquote_text_via_braced_parameter_still_fails() {
    expect_fail(
        "assign-backquote",
        "a=(x y z)\nx='`echo 1`'\na[${x}]=NEW\nrc=$?\ndeclare -p a\nexit \"$rc\"\n",
        BACKQUOTE_1,
    );
}

/// Guard: a direct backquote subscript still executes (word expansion)
/// and removes the element.
#[test]
fn unset_direct_backquote_valid_result_still_removes() {
    expect_ok(
        "direct-backquote",
        "a=(x y z)\nunset 'a[`echo 1`]'\nrc=$?\ndeclare -p a\nexit \"$rc\"\n",
        REMOVED_1,
    );
}

/// Guard: a direct `$(...)` subscript still executes.
#[test]
fn unset_direct_command_substitution_still_removes() {
    expect_ok(
        "direct-comsub",
        "a=(x y z)\nunset 'a[$(echo 1)]'\nrc=$?\ndeclare -p a\nexit \"$rc\"\n",
        REMOVED_1,
    );
}

/// Guard: a plain index still removes.
#[test]
fn unset_plain_index_still_removes() {
    expect_ok(
        "plain-index",
        "a=(x y z)\nunset 'a[1]'\nrc=$?\ndeclare -p a\nexit \"$rc\"\n",
        REMOVED_1,
    );
}

/// Guard: a direct `$y` subscript still expands (the single pass).
#[test]
fn unset_dollar_variable_still_removes() {
    expect_ok(
        "dollar-direct",
        "a=(x y z)\ny=1\nunset 'a[$y]'\nrc=$?\ndeclare -p a\nexit \"$rc\"\n",
        REMOVED_1,
    );
}

/// Guard: a bare variable NAME via `${x}` still evaluates through evalexp.
#[test]
fn unset_bare_name_via_braced_parameter_still_removes() {
    expect_ok(
        "bare-name-braced",
        "a=(x y z)\ni=1\nx='i'\nunset 'a[${x}]'\nrc=$?\ndeclare -p a\nexit \"$rc\"\n",
        REMOVED_1,
    );
}

/// Guard: a bare arithmetic expression via `${x}` still evaluates.
#[test]
fn unset_arithmetic_expression_via_braced_parameter_still_removes() {
    expect_ok(
        "arith-expr-braced",
        "a=(x y z)\ni=1\nx='i+1'\nunset 'a[${x}]'\nrc=$?\ndeclare -p a\nexit \"$rc\"\n",
        REMOVED_2,
    );
}

/// Guard: a plain arithmetic subscript still evaluates.
#[test]
fn unset_arithmetic_expression_subscript_still_removes() {
    expect_ok(
        "arith-expr-direct",
        "a=(x y z)\ni=1\nunset 'a[i+1]'\nrc=$?\ndeclare -p a\nexit \"$rc\"\n",
        REMOVED_2,
    );
}

/// Guard: arithmetic side effects in subscripts still apply.
#[test]
fn unset_side_effect_subscript_still_applies() {
    expect_ok(
        "side-effect",
        "a=(x y z)\ni=1\nunset 'a[i++]'\nrc=$?\necho \"i=$i\"\ndeclare -p a\nexit \"$rc\"\n",
        "i=2\ndeclare -a a=([0]=\"x\" [2]=\"z\")\n",
    );
}

/// Guard: a verbatim backquote through a nameref still executes (the
/// Protected single pass).
#[test]
fn unset_verbatim_backquote_through_nameref_still_removes() {
    expect_ok(
        "verbatim-nameref",
        "a=(x y z)\ndeclare -n n=a\nunset 'n[`echo 1`]'\nrc=$?\ndeclare -p a\nexit \"$rc\"\n",
        REMOVED_1,
    );
}

/// Guard: assignment with a plain variable subscript still stores.
#[test]
fn assign_plain_variable_still_stores() {
    expect_ok(
        "assign-plain",
        "a=(x y z)\nx=1\na[$x]=NEW\nrc=$?\ndeclare -p a\nexit \"$rc\"\n",
        "declare -a a=([0]=\"x\" [1]=\"NEW\" [2]=\"z\")\n",
    );
}

/// Guard: a direct backquote with invalid output still fails (word
/// expansion produces `1 2`, evalexp stops at `2`).
#[test]
fn unset_direct_backquote_invalid_output_still_fails() {
    expect_fail(
        "direct-backquote-invalid",
        "a=(x y z)\nunset 'a[`echo \"1 2\"`]'\nrc=$?\ndeclare -p a\nexit \"$rc\"\n",
        "1 2: arithmetic syntax error in expression (error token is \"2\")",
    );
}

/// Guard: after a failing unset the file resumes at the next line.
#[test]
fn unset_failure_then_next_unset_still_runs() {
    let (stdout, stderr, rc) = rubash_file(
        "continue-after-failure",
        "a=(x y z)\nunset 'a[`echo \"1 2\"`]'\nunset a[0]\nrc=$?\ndeclare -p a\nexit \"$rc\"\n",
    );
    assert_eq!(stderr.lines().count(), 1, "stderr lines: {stderr}");
    assert!(
        stderr.contains("1 2: arithmetic syntax error in expression (error token is \"2\")"),
        "stderr: {stderr}"
    );
    assert_eq!(rc, Some(0), "rc: stdout={stdout} stderr={stderr}");
    assert_eq!(
        stdout, "declare -a a=([1]=\"y\" [2]=\"z\")\n",
        "wrong stdout"
    );
}
