//! Issue rubash#1566: `${!PIPESTATUS[@]}` as the first touch of
//! PIPESTATUS reported `PIPESTATUS[@]: invalid indirect expansion`
//! and aborted the command (rc=1). The indirect-expansion error scan
//! judged the base variable's existence from env_vars / the variable
//! table only, but the PIPESTATUS array cell lives in shell_state
//! (variables.c:6291 set_pipestatus_array writes the ARRAY object
//! directly); a name with no cell in either table was treated as
//! unresolvable.
//!
//! GNU model: parameter_brace_expand (subst.c:10016-10047) dispatches
//! the pure `${!name[@]}` / `${!name[*]}` KEYS form to array_keys
//! BEFORE parameter_brace_expand_indir runs, so the keys form never
//! validates the base — an absent name yields the empty key list,
//! even under `set -u`. The plain and element indirection forms keep
//! their errors for genuinely unset names (subst.c:7911-7918,
//! 7924-7934), and PIPESTATUS itself is lazily created by the first
//! completed command, so a literal first-command `${!PIPESTATUS}`
//! errors in GNU too — pinned below as the matching edge.
//!
//! Expected outputs verified against GNU bash 5.3.20 (-c strings and
//! script files). Sibling gates: issue1363 (21) and issue1364 (12)
//! PIPESTATUS first-touch matrices must stay green.

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
fn issue1566_keys_first_touch_is_empty_not_error() {
    // No command has completed yet, so GNU's PIPESTATUS array does
    // not exist yet: the keys form is the empty list, rc=0 — never
    // the fatal "invalid indirect expansion".
    let (stdout, stderr, code) = rubash("echo \"K=[${!PIPESTATUS[@]}]\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "K=[]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1566_keys_star_first_touch() {
    let (stdout, stderr, code) = rubash("echo \"K=[${!PIPESTATUS[*]}]\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "K=[]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1566_keys_after_pipeline() {
    let (stdout, stderr, code) = rubash("true | false; echo \"K=[${!PIPESTATUS[@]}]\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "K=[0 1]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1566_keys_after_simple_command() {
    let (stdout, stderr, code) = rubash("echo hdr; echo \"K=[${!PIPESTATUS[@]}]\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "hdr\nK=[0]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1566_keys_unset_name_is_empty() {
    // The keys form on a wholly absent name: empty list, rc=0 — also
    // under `set -u`.
    let (stdout, stderr, code) = rubash("echo \"K=[${!no_such[@]}]\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "K=[]\n");
    assert_eq!(stderr, "");

    let (stdout, stderr, code) = rubash("set -u; echo \"K=[${!no_such[@]}]\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "K=[]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1566_plain_and_element_after_pipeline() {
    // After a pipeline the cell exists, so the plain and element
    // indirection forms resolve through element 0 (here "1", naming
    // the unset $1) instead of erroring.
    let (stdout, stderr, code) = rubash("false | true; echo \"P=[${!PIPESTATUS}]\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "P=[]\n");
    assert_eq!(stderr, "");

    let (stdout, stderr, code) = rubash("false | true; echo \"E=[${!PIPESTATUS[0]}]\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "E=[]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1566_plain_form_resolves_through_dollar_zero() {
    // After `true | false` element 0 is "0", so the plain form
    // re-expands the positional parameter $0 — the shell's own
    // invocation path (contains the binary name), not an empty
    // string. The numeric-target re-expansion is the mechanism:
    // index 0 designates $0, which lives outside the $1..
    // positional window; the same pin holds for an ordinary
    // variable whose value is "0".
    let (stdout, stderr, code) = rubash("true | false; echo \"P=[${!PIPESTATUS}]\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert!(
        stdout.starts_with("P=[") && stdout.contains("rubash"),
        "stdout: {stdout}"
    );

    let (stdout, stderr, code) = rubash("v=0; echo \"P=[${!v}]\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert!(
        stdout.starts_with("P=[") && stdout.contains("rubash"),
        "stdout: {stdout}"
    );
}

#[test]
fn issue1566_keys_in_command_substitution_first_touch() {
    let (stdout, stderr, code) = rubash("x=$(echo \"[${!PIPESTATUS[@]}]\"); echo \"[$x]\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "[[]]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1566_keys_controls_unchanged() {
    // Controls that already worked: declared-empty array keys and a
    // scalar's implicit [0] key list.
    let (stdout, stderr, code) = rubash("declare -a ea; echo \"K=[${!ea[@]}]\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "K=[]\n");

    let (stdout, stderr, code) = rubash("s=hi; echo \"K=[${!s[@]}]\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "K=[0]\n");
}

#[test]
fn issue1566_genuine_errors_preserved() {
    // The forms GNU really does reject must keep rejecting, byte for
    // byte in the diagnostic's name part: plain indirection on an
    // unset name, element indirection on an unset base, and the
    // keys-with-operator-tail form (not the pure keys form).
    let (_, stderr, code) = rubash("echo \"[${!no_such}]\"");
    assert_eq!(code, Some(1));
    assert!(
        stderr.contains("no_such: invalid indirect expansion"),
        "stderr: {stderr}"
    );

    let (_, stderr, code) = rubash("echo \"[${!no_such[0]}]\"");
    assert_eq!(code, Some(1));
    assert!(
        stderr.contains("no_such[0]: invalid indirect expansion"),
        "stderr: {stderr}"
    );

    let (_, stderr, code) = rubash("echo \"[${!no_such[@]:-d}]\"");
    assert_eq!(code, Some(1));
    assert!(
        stderr.contains("no_such[@]: invalid indirect expansion"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1566_operator_forms_after_pipeline() {
    // With the cell materialized, the operator forms adjudicate
    // through t = element 0 (naming $0, which is set) instead of
    // erroring on the base: `:-` keeps $0's value (the shell's own
    // invocation path — it contains the binary name and is neither
    // the default nor empty), `:+` fires.
    let (stdout, stderr, code) = rubash("true | false; echo \"X=[${!PIPESTATUS:-d}]\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert!(
        stdout.starts_with("X=[")
            && stdout.ends_with("]\n")
            && stdout.contains("rubash")
            && !stdout.contains("[d]"),
        "stdout: {stdout}"
    );

    let (stdout, stderr, code) = rubash("true | false; echo \"X=[${!PIPESTATUS:+y}]\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "X=[y]\n");
    assert_eq!(stderr, "");

    // The `[@]`-with-operator form is NOT the pure keys form: GNU
    // indirects through the element-value join "0 1", which is not a
    // valid variable name — the diagnostic names the join.
    let (_, stderr, code) = rubash("true | false; echo \"X=[${!PIPESTATUS[@]:-d}]\"");
    assert_eq!(code, Some(1));
    assert!(
        stderr.contains("0 1: invalid variable name"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1566_plain_first_touch_still_errors_like_gnu() {
    // GNU's lazy creation edge: before any command completes,
    // PIPESTATUS is genuinely unset and the PLAIN form errors in
    // both shells. Pinning it keeps the keys-form fix from
    // over-admitting the plain form.
    let (_, stderr, code) = rubash("echo \"[${!PIPESTATUS}]\"");
    assert_eq!(code, Some(1));
    assert!(
        stderr.contains("PIPESTATUS: invalid indirect expansion"),
        "stderr: {stderr}"
    );
}
