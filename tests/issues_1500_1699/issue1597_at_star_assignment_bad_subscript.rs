//! Issue rubash#1597: `=` / `:=` through the all-elements spelling
//! (`name[@]` / `name[*]`) skipped the bad-array-subscript validation
//! when the base was unset or a scalar: GNU parameter_brace_expand's
//! `case '='` (subst.c:10346) routes the still-subscripted name to
//! parameter_brace_expand_rhs (subst.c:8096), and the bind fails with
//! `name[@]: bad array subscript` (EX_BADUSAGE, status 2) in every
//! command position. The status the shell surfaces is mode-dependent:
//! a script file's reader_loop preserves the pre-set EX_BADUSAGE
//! (subst.c:8164; eval.c:111-118), while a `-c` string runs under the
//! parse_and_execute catch (builtins/evalstring.c:423-450), which
//! folds it to 1. This harness invokes `-c`, so the five
//! simple-command pins below assert rc=1; GNU gives 2 for the same
//! shapes from files, and rubash matches GNU in both modes (oracle
//! matrix in the rubash#2528 investigation). The one legal base is an associative array,
//! where the literal "@" / "*" text is an ordinary key. The same
//! issue's second root: the error operators (`:?` / `?`) on a BARE
//! array name read the raw storage string instead of element 0
//! (variables.c get_string_value; assoc: key "0"), so an assoc with
//! no "0" key or a sparse indexed array with no element 0 wrongly
//! passed the null test.
//!
//! Expected outputs verified against GNU bash 5.3.20 in both modes
//! (script files and `-c` strings); this harness uses `-c`.

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
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
fn issue1597_unset_at_assignment_is_bad_subscript() {
    let (stdout, stderr, code) = rubash("echo ${never_set_xyz[@]:=dv}\necho \"rc=$?\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("never_set_xyz[@]: bad array subscript"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1597_unset_star_assignment_plain_equals_is_bad_subscript() {
    let (stdout, stderr, code) = rubash("echo ${never_set_xyz[*]=dv}\necho \"rc=$?\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("never_set_xyz[*]: bad array subscript"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1597_scalar_null_at_assignment_is_bad_subscript() {
    let (stdout, stderr, code) = rubash("v=''\necho ${v[@]:=dv}\necho \"rc=$? v=[$v]\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1 v=[]\n");
    assert!(
        stderr.contains("v[@]: bad array subscript"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1597_scalar_set_at_assignment_does_not_trigger() {
    // A non-null scalar under [@] is VT_VARIABLE with a value, so
    // `:=` never reaches the bind: no error, scalar untouched.
    let (stdout, stderr, code) = rubash("v=hello\necho \"[${v[@]:=dv}] v=[$v]\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "[hello] v=[hello]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1597_empty_indexed_array_at_assignment_is_bad_subscript() {
    let (stdout, stderr, code) = rubash("a=()\necho ${a[@]:=dv}\necho \"rc=$?\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("a[@]: bad array subscript"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1597_single_empty_element_array_at_assignment_is_bad_subscript() {
    // The `:=` trigger test is the JOINED-elements null test: one
    // empty element joins to "" and still triggers the bind.
    let (stdout, stderr, code) = rubash("a=(\"\")\necho ${a[@]:=dv}\necho \"rc=$?\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("a[@]: bad array subscript"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1597_nonnull_array_at_assignment_does_not_trigger() {
    let (stdout, stderr, code) = rubash("a=(x \"\")\nj=${a[@]:=dv}\necho \"j=[$j]\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "j=[x ]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1597_assoc_at_assignment_binds_literal_at_key() {
    // The one legal base: on an (empty) associative array the
    // literal "@" text is an ordinary key and the bind succeeds.
    let (stdout, stderr, code) =
        rubash("declare -A m\nx=${m[@]:=dv}\necho \"at=[${m[\"@\"]}] x=[$x]\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "at=[dv] x=[dv]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1597_assoc_star_assignment_binds_literal_star_key() {
    let (stdout, stderr, code) =
        rubash("declare -A m\nx=${m[*]:=dv}\necho \"star=[${m[\"*\"]}] x=[$x]\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "star=[dv] x=[dv]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1597_bad_subscript_status_in_pipeline() {
    // rubash#2556: this harness runs `-c`, and GNU folds the element
    // DISCARD to 1 under `-c` (builtins/evalstring.c:423-450); the
    // file-mode value is 2.
    let (stdout, _, code) =
        rubash("echo ${never_set_xyz[@]:=dv} | cat\necho \"ps=${PIPESTATUS[0]}\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "ps=1\n");
}

#[test]
fn issue1597_bare_assoc_without_zero_key_fails_error_operator() {
    // Element 0 of an assoc is key "0"; without it the bare name is
    // null for `:?` even though other keys exist — fatal in GNU.
    let (stdout, stderr, code) = rubash("declare -A m=([k]=v)\n: ${m:?}\necho alive");
    assert_ne!(code, Some(0));
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("m: parameter null or not set"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1597_bare_sparse_indexed_without_element_zero_fails_error_operator() {
    let (stdout, stderr, code) = rubash("a=([1]=x)\n: ${a:?}\necho alive");
    assert_ne!(code, Some(0));
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("a: parameter null or not set"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1597_bare_assoc_with_zero_key_passes_error_operator() {
    let (stdout, _, code) = rubash("declare -A m=([0]=zero [k]=v)\n: ${m:?}\necho alive");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "alive\n");
}

#[test]
fn issue1597_bare_indexed_with_element_zero_passes_error_operator() {
    let (stdout, _, code) = rubash("a=(x)\n: ${a:?}\necho alive");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "alive\n");
}
