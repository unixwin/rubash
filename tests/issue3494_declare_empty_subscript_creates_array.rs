//! rubash#3494 — `declare 'a[]=x'` must create the array GNU creates
//! before assign_array_element rejects the empty subscript:
//! `declare -a a=()` (global), `declare -A a=()` under -A, an existing
//! scalar promoted to `[0]`, and the function-scope attribute-only form
//! `declare -a a`. The bare `a[]: bad array subscript` diagnostic and
//! failure status are unchanged (rubash#3186).
//!
//! Expectations verified against WSL GNU Bash 5.3.20 script-file probes
//! (scratch-h3/h3-3494-oracle*.sh); GNU anchors: declare.def:613
//! (valid_identifier on the bare name for valued operands),
//! declare.def:988-1011 (creation/VSETATTR before the element failure),
//! arrayfunc.c:1422 array_variable_name (`ni <= ind + 1` ->
//! err_badarraysub).

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
fn declare_empty_subscript_creates_empty_indexed_array() {
    let (stdout, stderr, _) = rubash("declare 'a[]=x'; declare -p a\n");
    assert_eq!(stdout, "declare -a a=()\n");
    assert!(
        stderr.contains("a[]: bad array subscript"),
        "stderr: {stderr}"
    );
}

#[test]
fn declare_dash_a_empty_subscript_creates_empty_array() {
    let (stdout, _, _) = rubash("declare -a 'a[]=x'; declare -p a\n");
    assert_eq!(stdout, "declare -a a=()\n");
}

#[test]
fn declare_dash_cap_a_empty_subscript_creates_empty_assoc() {
    let (stdout, _, _) = rubash("declare -A 'a[]=x'; declare -p a\n");
    assert_eq!(stdout, "declare -A a=()\n");
}

#[test]
fn declare_empty_subscript_append_creates_empty_array() {
    let (stdout, _, _) = rubash("declare 'a[]+=x'; declare -p a\n");
    assert_eq!(stdout, "declare -a a=()\n");
}

#[test]
fn typeset_empty_subscript_creates_empty_array() {
    let (stdout, _, _) = rubash("typeset 'a[]=x'; declare -p a\n");
    assert_eq!(stdout, "declare -a a=()\n");
}

#[test]
fn declare_empty_subscript_status_is_failure() {
    let (stdout, _, code) = rubash("declare 'a[]=x'; echo rc=$?\n");
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(code, Some(0));
}

#[test]
fn declare_empty_subscript_continues_to_later_operands() {
    let (stdout, _, _) = rubash("declare 'a[]=x' b=2; declare -p a b\n");
    assert_eq!(stdout, "declare -a a=()\ndeclare -- b=\"2\"\n");
}

#[test]
fn declare_empty_subscript_promotes_existing_scalar() {
    let (stdout, _, _) = rubash("a=5; declare 'a[]=x'; declare -p a\n");
    assert_eq!(stdout, "declare -a a=([0]=\"5\")\n");
}

#[test]
fn declare_empty_subscript_keeps_existing_assoc() {
    let (stdout, _, _) = rubash("declare -A a; declare 'a[]=x'; declare -p a\n");
    assert_eq!(stdout, "declare -A a\n");
}

#[test]
fn declare_empty_subscript_keeps_existing_indexed() {
    let (stdout, _, _) = rubash("declare -a a; declare 'a[]=x'; declare -p a\n");
    assert_eq!(stdout, "declare -a a\n");
}

#[test]
fn declare_empty_subscript_scalar_to_assoc_moves_value_to_zero() {
    let (stdout, _, _) = rubash("a=5; declare -A 'a[]=x'; declare -p a\n");
    assert_eq!(stdout, "declare -A a=([0]=\"5\" )\n");
}

#[test]
fn declare_empty_subscript_empty_scalar_to_assoc_keeps_empty_element() {
    let (stdout, _, _) = rubash("a=; declare -A 'a[]=x'; declare -p a\n");
    assert_eq!(stdout, "declare -A a=([0]=\"\" )\n");
}

#[test]
fn local_empty_subscript_creates_attribute_only_local() {
    let (stdout, _, _) = rubash("f() { local 'a[]=x'; declare -p a; }; f\n");
    assert_eq!(stdout, "declare -a a\n");
}

#[test]
fn declare_in_function_empty_subscript_is_attribute_only() {
    let (stdout, _, _) = rubash("f() { declare 'a[]=x'; declare -p a; }; f\n");
    assert_eq!(stdout, "declare -a a\n");
}

#[test]
fn local_dash_cap_a_empty_subscript_is_attribute_only() {
    let (stdout, _, _) = rubash("f() { local -A 'a[]=x'; declare -p a; }; f\n");
    assert_eq!(stdout, "declare -A a\n");
}

#[test]
fn declare_empty_subscript_readonly_base_reports_readonly() {
    let (stdout, stderr, _) = rubash("declare -r a=5; declare 'a[]=x'; declare -p a\n");
    assert_eq!(stdout, "declare -r a=\"5\"\n");
    assert!(
        stderr.contains("declare: a: readonly variable"),
        "stderr: {stderr}"
    );
    assert!(
        !stderr.contains("bad array subscript"),
        "readonly must pre-empt badsub, stderr: {stderr}"
    );
}

#[test]
fn declare_empty_subscript_readonly_array_reports_readonly() {
    let (stdout, stderr, _) = rubash("declare -ar a; declare 'a[]=x'; declare -p a\n");
    assert_eq!(stdout, "declare -ar a\n");
    assert!(
        stderr.contains("declare: a: readonly variable"),
        "stderr: {stderr}"
    );
}

#[test]
fn declare_empty_subscript_drops_deferred_nameref() {
    let (stdout, _, _) = rubash("declare -n 'a[]=x'; declare -p a\n");
    assert_eq!(stdout, "declare -a a=()\n");
}

#[test]
fn declare_bare_empty_subscript_stays_invalid_id() {
    let (_, stderr, _) = rubash("declare 'a[]'\n");
    assert!(
        stderr.contains("not a valid identifier"),
        "stderr: {stderr}"
    );
}

#[test]
fn export_empty_subscript_stays_invalid_id() {
    let (_, stderr, _) = rubash("export 'a[]=x'\n");
    assert!(
        stderr.contains("not a valid identifier"),
        "stderr: {stderr}"
    );
}

#[test]
fn readonly_empty_subscript_stays_invalid_id() {
    let (_, stderr, _) = rubash("readonly 'a[]=x'\n");
    assert!(
        stderr.contains("not a valid identifier"),
        "stderr: {stderr}"
    );
}

#[test]
fn declare_p_empty_subscript_stays_not_found() {
    let (stdout, stderr, _) = rubash("declare -p 'a[]=x'\n");
    assert_eq!(stdout, "");
    assert!(stderr.contains("a[]=x: not found"), "stderr: {stderr}");
}
