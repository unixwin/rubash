//! Issue rubash#1569: listing builtins printed the tempenv PREFIX
//! bindings of the very command executing the listing —
//! `zzq=1 export -p` listed `declare -x zzq="1"`.
//!
//! GNU model: listings iterate the real variable contexts
//! (all_shell_variables / print_var_list, variables.c); a
//! `name=value cmd` binding lives in the separate temporary_env
//! table and reaches a listing only when it was pushed into a
//! context — a function call (push_context, execute_cmd.c:5233),
//! eval/source (push_scope VC_BLTNENV, execute_cmd.c:5047), a
//! subshell fork, or the posix special-builtin merge. A listing
//! executed by the carrying command itself therefore shows the
//! binding UNDERNEATH the prefix frame: nothing for a name the
//! frame created, the pre-frame value/attributes for a shadowed
//! name. Explicit-name lookups still see the tempvar
//! (find_variable searches temporary_env, variables.c:1980), and
//! external children still receive the binding through the
//! environment (maybe_make_export_env).
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
fn issue1569_own_prefix_export_p_hides_the_tempvar() {
    let (stdout, stderr, code) = rubash("zzq=1 export -p");
    assert_eq!(code, Some(0));
    assert!(!stdout.contains("zzq"), "stdout: {stdout}");
    assert_eq!(stderr, "");
}

#[test]
fn issue1569_own_prefix_export_p_shows_the_underlying_value() {
    let (stdout, stderr, code) = rubash("export zzq=0\nzzq=1 export -p | grep zzq");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -x zzq=\"0\"\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1569_own_prefix_declare_p_list() {
    let (stdout, stderr, code) = rubash("zzq=1 declare -p");
    assert_eq!(code, Some(0));
    // (The -c string itself mentions zzq via BASH_EXECUTION_STRING;
    // what must not appear is a declaration OF zzq.)
    assert!(!stdout.contains("zzq=\"1\""), "stdout: {stdout}");
    assert_eq!(stderr, "");

    // The grep pattern anchors on the declaration line itself so
    // the BASH_EXECUTION_STRING entry (which quotes the -c text in
    // both shells) cannot false-positive it.
    let (stdout, stderr, code) = rubash("zzq=0\nzzq=1 declare -p | grep '^declare -- zzq='");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -- zzq=\"0\"\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1569_own_prefix_bare_declare_and_set() {
    // `^zzq=` anchors at the assignment-form line start so the
    // BASH_EXECUTION_STRING entry (which quotes the -c text) cannot
    // false-positive the grep.
    let (stdout, _, code) = rubash("zzq=1 declare | grep ^zzq=\necho rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\n");

    let (stdout, _, code) = rubash("zzq=1 set | grep ^zzq=\necho rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\n");
}

#[test]
fn issue1569_own_prefix_readonly_p() {
    let (stdout, _, code) = rubash("zzq=1 readonly -p | grep zzq\necho rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\n");

    // A readonly underlying binding renders with its own value;
    // the prefix word itself is dropped with the readonly
    // diagnostic (assign_in_env's ASSIGN_DISALLOWED arm).
    let (stdout, stderr, code) = rubash("readonly rz=0\nrz=1 readonly -p | grep rz");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -r rz=\"0\"\n");
    assert!(stderr.contains("rz: readonly variable"), "stderr: {stderr}");
}

#[test]
fn issue1569_own_prefix_compgen() {
    let (stdout, _, code) = rubash("zzq=1 compgen -e | grep -x zzq\necho rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\n");

    // An exported underlying binding still completes.
    let (stdout, stderr, code) = rubash("export zzq=0\nzzq=1 compgen -e | grep -x zzq");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "zzq\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1569_outer_frames_stay_visible() {
    // Function frame: the binding was pushed into the callee's
    // context, so listings inside see it.
    let (stdout, stderr, code) = rubash("f() { export -p | grep zzq; }\nzzq=1 f");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -x zzq=\"1\"\n");
    assert_eq!(stderr, "");

    // Eval frame: same push through the VC_BLTNENV scope.
    let (stdout, stderr, code) = rubash("zzq=1 eval 'export -p | grep zzq'");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -x zzq=\"1\"\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1569_nested_frames_discriminate() {
    // zzq rides the eval frame (visible); wq is the listing
    // command's OWN prefix (hidden).
    let (stdout, stderr, code) = rubash("zzq=1 eval 'wq=2 export -p | grep -E \"zzq|wq\"'");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -x zzq=\"1\"\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1569_explicit_name_lookup_still_sees_the_tempvar() {
    let (stdout, stderr, code) = rubash("zzq=1 declare -p zzq");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -x zzq=\"1\"\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1569_external_child_still_receives_the_binding() {
    let (stdout, stderr, code) = rubash("zzq=1 env | grep ^zzq=");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "zzq=1\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1569_command_prefix_form() {
    let (stdout, _, code) = rubash("zzq=1 command export -p | grep zzq\necho rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\n");
}

#[test]
fn issue1569_pipestatus_prefix_renders_the_underlying_shape() {
    let (stdout, stderr, code) = rubash("false | true\nPIPESTATUS=99 declare -p | grep PIPESTATUS");
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("PIPESTATUS=([0]=\"1\" [1]=\"0\")"),
        "stdout: {stdout}"
    );
    // (Not a bare "99" check: BASH_EXECUTION_STRING quotes the -c
    // text, which legitimately contains the prefix word.)
    assert!(!stdout.contains("=\"99\""), "stdout: {stdout}");
    assert_eq!(stderr, "");
}
