//! Issue rubash#1556: a plain temporary assignment over an array
//! name bound element 0 in place, so reads inside the tempenv
//! window saw a mutated array (`a=(9 9); a=99 declare -p a`
//! printed `declare -ax a=([0]="99" [1]="9")`).
//!
//! GNU model (variables.c:3536 assign_in_env, called from
//! subst.c:13142 do_assignment_statements): every tempenv binding
//! is a fresh scalar created in the separate temporary_env table
//! (make_new_variable + var_setvalue, att_exported|att_tempvar
//! only), shadowing the outer cell whole; the outer cell is never
//! written, so post-command state is trivially intact. Window
//! reads of every shape (declare -p, element/subscript reads,
//! ${a[@]}, ${!a[@]}, ${#a[@]}, arithmetic key evaluation on an
//! assoc shadow) see the scalar. Append computes against the old
//! cell's scalar view (make_variable_value with ASS_APPEND:
//! element 0 for indexed arrays, empty for assoc arrays).
//!
//! Expected outputs verified against GNU bash 5.3.20 (script
//! files).

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
fn issue1556_issue_repro_shadow_and_restore() {
    let (stdout, stderr, code) = rubash("a=(9 9)\na=99 declare -p a\ndeclare -p a");
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "declare -x a=\"99\"\ndeclare -a a=([0]=\"9\" [1]=\"9\")\n"
    );
    assert_eq!(stderr, "");
}

#[test]
fn issue1556_window_reads_see_the_scalar() {
    let (stdout, stderr, code) = rubash(
        "a=(9 9)\na=99 eval 'echo \"w0=[${a[0]}] w1=[${a[1]}] w@=[${a[@]}] widx=[${!a[@]}] wlen=[${#a[@]}]\"'",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "w0=[99] w1=[] w@=[99] widx=[0] wlen=[1]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1556_function_and_typeset_entries() {
    let (stdout, stderr, code) =
        rubash("a=(9 9)\nf() { declare -p a; }\na=99 f\na=99 typeset -p a");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -x a=\"99\"\ndeclare -x a=\"99\"\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1556_compound_window_is_literal_scalar() {
    let (stdout, stderr, code) = rubash("a=(9 9)\na=(7 8) declare -p a\ndeclare -p a");
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "declare -x a=\"(7 8)\"\ndeclare -a a=([0]=\"9\" [1]=\"9\")\n"
    );
    assert_eq!(stderr, "");
}

#[test]
fn issue1556_append_uses_scalar_view() {
    let (stdout, stderr, code) = rubash("a=(9 9)\na+=5 declare -p a\ndeclare -p a");
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "declare -x a=\"95\"\ndeclare -a a=([0]=\"9\" [1]=\"9\")\n"
    );
    assert_eq!(stderr, "");

    let (stdout, stderr, code) =
        rubash("declare -A A=([x]=1 [y]=2)\nA+=5 declare -p A\ndeclare -p A");
    assert_eq!(code, Some(0));
    // The window line is exact; the restored assoc line is checked
    // by fragments because assoc rendering order is hash order in
    // both shells.
    let mut lines = stdout.lines();
    assert_eq!(lines.next(), Some("declare -x A=\"5\""));
    let restored = lines.next().unwrap_or_default();
    assert!(restored.starts_with("declare -A A=("), "stdout: {stdout}");
    assert!(restored.contains("[x]=\"1\""), "stdout: {stdout}");
    assert!(restored.contains("[y]=\"2\""), "stdout: {stdout}");
    assert_eq!(stderr, "");
}

#[test]
fn issue1556_assoc_window_and_key_reads() {
    let (stdout, stderr, code) =
        rubash("declare -A A=([x]=1 [y]=2)\nA=99 declare -p A\ndeclare -p A");
    assert_eq!(code, Some(0));
    let mut lines = stdout.lines();
    assert_eq!(lines.next(), Some("declare -x A=\"99\""));
    let restored = lines.next().unwrap_or_default();
    assert!(restored.starts_with("declare -A A=("), "stdout: {stdout}");
    assert!(restored.contains("[x]=\"1\""), "stdout: {stdout}");
    assert!(restored.contains("[y]=\"2\""), "stdout: {stdout}");
    assert_eq!(stderr, "");

    // On the scalar shadow every key evaluates arithmetically, so
    // any key reads the shadow value.
    let (stdout, stderr, code) =
        rubash("declare -A A=([x]=1)\nA=99 eval 'echo \"wx=[${A[x]}] w0=[${A[0]}]\"'");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "wx=[99] w0=[99]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1556_nameref_target_is_shadowed() {
    let (stdout, stderr, code) =
        rubash("a=(9 9)\ndeclare -n r=a\nr=99 declare -p a r\ndeclare -p a");
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "declare -x a=\"99\"\ndeclare -n r=\"a\"\ndeclare -a a=([0]=\"9\" [1]=\"9\")\n"
    );
    assert_eq!(stderr, "");
}

#[test]
fn issue1556_double_word_and_empty_value() {
    let (stdout, stderr, code) = rubash("a=(9 9)\na=1 a=2 declare -p a\ndeclare -p a");
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "declare -x a=\"2\"\ndeclare -a a=([0]=\"9\" [1]=\"9\")\n"
    );
    assert_eq!(stderr, "");

    let (stdout, stderr, code) = rubash("a=(9 9)\na= declare -p a");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -x a=\"\"\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1556_integer_array_window_keeps_raw_text() {
    // The tempvar carries no integer attribute, so no evaluation
    // runs on the window value.
    let (stdout, stderr, code) = rubash("declare -ai a=(9 9)\na=007 declare -p a\ndeclare -p a");
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "declare -x a=\"007\"\ndeclare -ai a=([0]=\"9\" [1]=\"9\")\n"
    );
    assert_eq!(stderr, "");
}

#[test]
fn issue1556_local_array_and_promotion() {
    let (stdout, stderr, code) =
        rubash("h() { local -a la=(3 4); la=99 declare -p la; declare -p la; }\nh");
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "declare -x la=\"99\"\ndeclare -a la=([0]=\"3\" [1]=\"4\")\n"
    );
    assert_eq!(stderr, "");

    // `declare a` inside the function promotes the SHADOW value to
    // a frame local; the global array is untouched afterwards.
    let (stdout, stderr, code) =
        rubash("g() { a=99 declare a; declare -p a; }\na=(9 9)\ng\ndeclare -p a");
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "declare -x a=\"99\"\ndeclare -a a=([0]=\"9\" [1]=\"9\")\n"
    );
    assert_eq!(stderr, "");
}

#[test]
fn issue1556_window_writes_do_not_leak() {
    let (stdout, stderr, code) = rubash("a=(9 9)\na=99 eval 'a=111 declare -p a'\ndeclare -p a");
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "declare -x a=\"111\"\ndeclare -a a=([0]=\"9\" [1]=\"9\")\n"
    );
    assert_eq!(stderr, "");
}

#[test]
fn issue1556_readonly_array_window() {
    // The rejected bind reports and the command still runs; the
    // window shows the untouched array with no export leak.
    let (stdout, stderr, code) = rubash("a=(9 9)\nreadonly a\na=99 declare -p a\necho \"rc=$?\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -ar a=([0]=\"9\" [1]=\"9\")\nrc=0\n");
    assert!(stderr.contains("a: readonly variable"), "stderr: {stderr}");
}

#[test]
fn issue1556_element_prefix_rejection_and_mixed_words() {
    // A lone element word is rejected by assign_in_env
    // (valid_identifier fails) and the command still runs with
    // the array untouched.
    let (stdout, stderr, code) = rubash("a=(9 9)\na[0]=5 declare -p a\necho \"rc=$?\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -a a=([0]=\"9\" [1]=\"9\")\nrc=0\n");
    assert!(
        stderr.contains("`a[0]': not a valid identifier"),
        "stderr: {stderr}"
    );

    // An invalid word does not stop the following simple
    // assignment words from binding (do_assignment_statements
    // walks every prefix word).
    let (stdout, stderr, code) = rubash("a=(9 9)\na[0]=5 b=2 declare -p b\necho \"rc=$?\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -x b=\"2\"\nrc=0\n");
    assert!(
        stderr.contains("`a[0]': not a valid identifier"),
        "stderr: {stderr}"
    );
}
