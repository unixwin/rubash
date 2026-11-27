//! Issue rubash#1568: two gaps in the array-element assignment
//! prefix handling.
//!
//! (1) A MIXED assignment statement with no command word —
//! `a[0]=5 b=2` — was not covered: the invalidid strip harvested
//! following plain words only when a command word remained, so
//! `b=2` was dispatched as a command name (127, nothing bound).
//! GNU assignment() (general.c:480) classifies BOTH word shapes as
//! assignment words; with no command word the node is an
//! assignment statement and every word binds permanently, in
//! order, with NO invalidid diagnostic (that diagnostic exists
//! only on the tempenv path, assign_in_env variables.c:3536).
//!
//! (2) Pipeline stages never passed through the strip: a stage
//! carrying an element prefix (`a[0]=5 b=2 env | cat`) dispatched
//! `a[0]=5` as a command name. GNU classifies each element's
//! words independently inside the forked element
//! (execute_cmd.c execute_pipeline -> execute_simple_command);
//! the diagnostic is reported during the element's word expansion,
//! before its own redirections apply, so it reaches the ambient
//! stderr even under `2>/dev/null` on the element.
//!
//! Expected outputs verified against GNU bash 5.3.20 (script
//! files, isolated per case).

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
fn issue1568_mixed_statement_binds_every_word() {
    let (stdout, stderr, code) = rubash("a[0]=5 b=2\ndeclare -p a b");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -a a=([0]=\"5\")\ndeclare -- b=\"2\"\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1568_mixed_statement_plain_word_first() {
    // The parser peels the leading plain word into cmd.assignments;
    // it must bind too (it was silently dropped before).
    let (stdout, stderr, code) = rubash("b=2 a[0]=5\ndeclare -p a b");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -a a=([0]=\"5\")\ndeclare -- b=\"2\"\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1568_mixed_statement_interleaved() {
    let (stdout, stderr, code) = rubash("a[0]=5 b=2 a[2]=9\ndeclare -p a b");
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "declare -a a=([0]=\"5\" [2]=\"9\")\ndeclare -- b=\"2\"\n"
    );
    assert_eq!(stderr, "");
}

#[test]
fn issue1568_mixed_statement_in_function_body() {
    let (stdout, stderr, code) = rubash("f() { a[0]=5 b=2; }\nf\ndeclare -p a b");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -a a=([0]=\"5\")\ndeclare -- b=\"2\"\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1568_mixed_statement_with_redirect() {
    let (stdout, stderr, code) = rubash("a[0]=5 b=2 >/dev/null\necho rc=$?\ndeclare -p b");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0\ndeclare -- b=\"2\"\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1568_mixed_statement_compound_plain_word() {
    let (stdout, stderr, code) = rubash("a[0]=5 b=(1 2)\ndeclare -p a b");
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "declare -a a=([0]=\"5\")\ndeclare -a b=([0]=\"1\" [1]=\"2\")\n"
    );
    assert_eq!(stderr, "");
}

#[test]
fn issue1568_command_form_still_diagnoses_and_binds() {
    let (stdout, stderr, code) = rubash("a[0]=5 b=2 declare -p b");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -x b=\"2\"\n");
    assert!(
        stderr.contains("`a[0]': not a valid identifier"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1568_command_form_invalid_word_not_first() {
    let (stdout, stderr, code) = rubash("x[9]=1 y=3 z=4 env | grep -E '^(y|z)='");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "y=3\nz=4\n");
    assert!(
        stderr.contains("`x[9]': not a valid identifier"),
        "stderr: {stderr}"
    );
    assert!(!stderr.contains("`y'"), "stderr: {stderr}");
}

#[test]
fn issue1568_stage_command_form() {
    let (stdout, stderr, code) = rubash("a[0]=5 b=2 env | grep ^b=");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "b=2\n");
    assert!(
        stderr.contains("`a[0]': not a valid identifier"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1568_stage_diagnostic_is_ambient() {
    // The diagnostic belongs to the expansion phase: the element's
    // own 2>/dev/null must not swallow it.
    let (stdout, stderr, code) = rubash("a[0]=5 b=2 env 2>/dev/null | grep ^b=");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "b=2\n");
    assert!(
        stderr.contains("`a[0]': not a valid identifier"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1568_stage_statement_binds_stage_locally() {
    let (stdout, stderr, code) = rubash("a[0]=5 b=2 | cat\necho done\ndeclare -p b");
    // The script's last command is the failing `declare -p b`
    // (the stage's binds must not leak) — GNU exits 1 as well.
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "done\n");
    assert!(stderr.contains("not found"), "stderr: {stderr}");
    assert!(!stderr.contains("command not found"), "stderr: {stderr}");
    assert!(
        !stderr.contains("not a valid identifier"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1568_comsub_command_form() {
    let (stdout, stderr, code) = rubash("x=$(a[0]=5 b=2 env | grep ^b=)\necho \"x=$x\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "x=b=2\n");
    assert!(
        stderr.contains("`a[0]': not a valid identifier"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1568_pure_element_statement_unchanged() {
    let (stdout, stderr, code) = rubash("a[0]=5 a[1]=6\ndeclare -p a");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -a a=([0]=\"5\" [1]=\"6\")\n");
    assert_eq!(stderr, "");
}
