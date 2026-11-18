//! Issue rubash#3522 (#3493 residual 2): `unset -v arr["$k"]` on an
//! INDEXED array with a backtick key silently succeeded (rc 0) while GNU
//! reports `` `arr[`]': not a valid identifier `` (rc 1). The expanded
//! operand is `arr[`]`; GNU's `tokenize_array_reference`
//! (arrayfunc.c:1318) runs the flag-0 `skipsubscript` scan
//! (subst.c:2086-2177), where a backquote opens a command-substitution
//! span that never closes, so no balance-0 `]` exists and the reference
//! is invalid. Rubash's validity gate (`valid_array_reference_env`,
//! flag-0 branch) scans with `scan_compound_subscript`, which never
//! opens a backquote span, so the gate passed; the element path's
//! quote-aware `parse_array_subscript` then failed, and the operand
//! fell through to the lib re-scan, which silently succeeds for an
//! existing array (`parse_unset_subscript` is quote-blind) and
//! misdiagnoses a scalar as "not an array variable".
//!
//! GNU oracle (bash 5.2.21 probes 2026-11-16, same version as the #3493
//! box probes): P1/P4/P9/P11/P13 all print
//! `` `name[`]': not a valid identifier `` with rc 1 and the variable
//! intact; without `-v` (P2) GNU stays silent with rc 0; a BALANCED
//! backquote span (P10) is a live command substitution and still
//! unsets element 0; the assoc face (P6) still unsets with rc 0.
//!
//! Fix (in `execute_unset_with_stderr`): the gate additionally requires
//! the quote-aware `parse_array_subscript` to succeed whenever GNU
//! would run the flag-0 scan - every case except an assoc base under
//! VA_NOEXPAND, where GNU takes the first `]` verbatim
//! (arrayfunc.c:1323-1324).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.arg0("bash");
    }
    let output = command
        .arg("-c")
        .arg(script)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("run rubash");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    // std exposes no argv[0] override on Windows; normalize the harness
    // exe-path token to `bash` in captured stderr, the same
    // normalization the GNU suites apply to the baseline's path
    // (see tests/issue2749_arith_for_attribution_regressions.rs).
    #[cfg(windows)]
    let stderr = {
        let exe = env!("CARGO_BIN_EXE_rubash");
        let slashed = exe.replace('\\', "/");
        let mut normalized = stderr.replace(exe, "bash").replace(&slashed, "bash");
        if let Some(drive) = slashed.chars().next().filter(|c| c.is_ascii_alphabetic()) {
            if slashed.as_bytes().get(1) == Some(&b':') {
                let posix = format!("/{}/{}", drive.to_ascii_lowercase(), &slashed[3..]);
                normalized = normalized.replace(&posix, "bash");
            }
        }
        normalized
    };
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr,
        output.status.code(),
    )
}

/// The issue repro, verbatim: indexed array + backtick key via a
/// double-quoted subscript. GNU: invalid identifier, rc 1, intact.
/// (Pre-fix rubash: silent rc 0.)
#[test]
fn unset_indexed_backtick_key_via_double_quoted_subscript() {
    let (stdout, stderr, _) = rubash(
        r#"declare -a arr; arr=(a b c)
k='`'
unset -v arr["$k"]; echo "rc=$?"
declare -p arr"#,
    );
    assert_eq!(
        stderr, "bash: line 3: unset: `arr[`]': not a valid identifier\n",
        "wrong stderr"
    );
    assert_eq!(
        stdout,
        "rc=1\ndeclare -a arr=([0]=\"a\" [1]=\"b\" [2]=\"c\")\n"
    );
}

/// Same key via unquoted expansion: also a W_ARRAYREF word, GNU
/// still rejects it. (Pre-fix rubash: silent rc 0.)
#[test]
fn unset_indexed_backtick_key_via_unquoted_expansion() {
    let (stdout, stderr, _) = rubash(
        r#"declare -a arr; arr=(a b c)
k='`'
unset -v arr[$k]; echo "rc=$?"
declare -p arr"#,
    );
    assert_eq!(
        stderr, "bash: line 3: unset: `arr[`]': not a valid identifier\n",
        "wrong stderr"
    );
    assert_eq!(
        stdout,
        "rc=1\ndeclare -a arr=([0]=\"a\" [1]=\"b\" [2]=\"c\")\n"
    );
}

/// The same lone backtick as a literal (non-W_ARRAYREF) operand:
/// GNU rejects it too - the gate, not the arrayref flag, is the fix.
/// (Pre-fix rubash: silent rc 0.)
#[test]
fn unset_indexed_backtick_key_literal_operand() {
    let (stdout, stderr, _) = rubash(
        r#"declare -a arr; arr=(a b c)
unset -v 'arr[`]'; echo "rc=$?"
declare -p arr"#,
    );
    assert_eq!(
        stderr, "bash: line 2: unset: `arr[`]': not a valid identifier\n",
        "wrong stderr"
    );
    assert_eq!(
        stdout,
        "rc=1\ndeclare -a arr=([0]=\"a\" [1]=\"b\" [2]=\"c\")\n"
    );
}

/// Scalar base: GNU reports the same invalid identifier, not the
/// lib's "not an array variable".
/// (Pre-fix rubash: rc 1 but `unset: v: not an array variable`.)
#[test]
fn unset_scalar_backtick_key_still_invalid() {
    let (stdout, stderr, _) = rubash(
        r#"v=hello
k='`'
unset -v v["$k"]; echo "rc=$?"
declare -p v"#,
    );
    assert_eq!(
        stderr, "bash: line 3: unset: `v[`]': not a valid identifier\n",
        "wrong stderr"
    );
    assert_eq!(stdout, "rc=1\ndeclare -- v=\"hello\"\n");
}

/// Without `-v` GNU treats the invalid name as a potential function
/// name and stays silent with rc 0 - the gate must not diagnose.
#[test]
fn unset_indexed_backtick_key_without_v_flag_silent() {
    let (stdout, stderr, _) = rubash(
        r#"declare -a arr; arr=(a b c)
k='`'
unset arr["$k"]; echo "rc=$?"
declare -p arr"#,
    );
    assert_eq!(stderr, "", "stderr not empty: {stderr}");
    assert_eq!(
        stdout,
        "rc=0\ndeclare -a arr=([0]=\"a\" [1]=\"b\" [2]=\"c\")\n"
    );
}

/// Through a nameref to an indexed array: GNU rejects the operand
/// itself (`n[`]`), rc 1. (Pre-fix rubash: silent rc 0.)
#[test]
fn unset_nameref_indexed_backtick_key_still_invalid() {
    let (stdout, stderr, _) = rubash(
        r#"declare -a arr; arr=(a b c); declare -n n=arr
k='`'
unset -v n["$k"]; echo "rc=$?"
declare -p arr"#,
    );
    assert_eq!(
        stderr, "bash: line 3: unset: `n[`]': not a valid identifier\n",
        "wrong stderr"
    );
    assert_eq!(
        stdout,
        "rc=1\ndeclare -a arr=([0]=\"a\" [1]=\"b\" [2]=\"c\")\n"
    );
}

/// A whole-word double-quoted operand is NOT a W_ARRAYREF word, so
/// GNU rejects it too.
#[test]
fn unset_indexed_backtick_key_whole_word_quoted_still_invalid() {
    let (stdout, stderr, _) = rubash(
        r#"declare -a arr; arr=(a b c)
k='`'
unset -v "arr[$k]"; echo "rc=$?"
declare -p arr"#,
    );
    assert_eq!(
        stderr, "bash: line 3: unset: `arr[`]': not a valid identifier\n",
        "wrong stderr"
    );
    assert_eq!(
        stdout,
        "rc=1\ndeclare -a arr=([0]=\"a\" [1]=\"b\" [2]=\"c\")\n"
    );
}

/// Guard: a BALANCED backquote span is a live command substitution,
/// so the subscript still evaluates and element 0 goes (GNU P10).
#[test]
fn unset_indexed_balanced_backtick_subscript_still_works() {
    let (stdout, stderr, _) = rubash(
        r#"declare -a arr; arr=(a b c)
unset -v 'arr[`0`]'; echo "rc=$?"
declare -p arr"#,
    );
    assert_eq!(
        stderr, "bash: line 2: 0: command not found\n",
        "wrong stderr"
    );
    assert_eq!(stdout, "rc=0\ndeclare -a arr=([1]=\"b\" [2]=\"c\")\n");
}

/// Guard: the assoc face keeps working - the backtick key unsets
/// with rc 0 (#2657, GNU P6).
#[test]
fn unset_assoc_backtick_key_still_unsets() {
    let (stdout, stderr, _) = rubash(
        r#"declare -A dict; dict['`']=2
k='`'
unset -v dict["$k"]; echo "rc=$?"
declare -p dict"#,
    );
    assert_eq!(stderr, "", "stderr not empty: {stderr}");
    assert_eq!(stdout, "rc=0\ndeclare -A dict=()\n");
}
