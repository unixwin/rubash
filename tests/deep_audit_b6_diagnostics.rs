//! Deep-audit Round 2, Lane B6 — diagnostic & exit-status fidelity.
//!
//! Each test names the finding it pins. Expected outputs are byte-verified
//! against GNU Bash 5.3.20 (the vendored oracle binary), with the owning
//! GNU C source cited per finding.

use std::io::Write;
use std::process::{Command, Stdio};

fn rubash_c(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash -c");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("b6-diag-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("case.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .output()
        .expect("run rubash file");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn rubash_stdin(script: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-s")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash -s");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(script.as_bytes())
        .expect("write script");
    let output = child.wait_with_output().expect("run rubash -s");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// B6-F1: a dangling `&&`/`||` at end of input is GNU's plain
/// "syntax error: unexpected end of file" (parse.y yyerror) at the
/// end-of-input line — never the "near unexpected end of file" form.
#[test]
fn b6_f1_dangling_and_or_uses_eof_form() {
    let (_out, err, code) = rubash_c("echo hi &&");
    assert_eq!(code, Some(2));
    assert!(
        err.contains("line 2: syntax error: unexpected end of file"),
        "stderr was: {err}"
    );
    assert!(!err.contains("near unexpected"), "stderr was: {err}");

    let (_out, err, code) = rubash_c("echo hi ||");
    assert_eq!(code, Some(2));
    assert!(
        err.contains("syntax error: unexpected end of file"),
        "stderr was: {err}"
    );
}

/// B6-F1: a multi-command list ending in a dangling connector is
/// rejected wholesale — GNU runs none of it.
#[test]
fn b6_f1_dangling_and_or_multi_command_list_rejected() {
    let (out, err, code) = rubash_file("echo a &&\necho b &&\n");
    assert_eq!(code, Some(2));
    assert_eq!(out, "", "no list command may run, stdout was: {out}");
    assert!(
        err.contains("syntax error: unexpected end of file"),
        "stderr was: {err}"
    );

    let (out, _err, code) = rubash_stdin("echo a &&\necho b &&\n");
    assert_eq!(code, Some(2));
    assert_eq!(out, "", "stdin route: stdout was: {out}");
}

/// B6-F2: `times` diagnostics carry the shell diagnostic prefix
/// (builtins/common.c builtin_error), not a hardcoded program name.
#[test]
fn b6_f2_times_diagnostic_prefix() {
    let (_out, err, code) = rubash_c("times -z; echo rc=$?");
    assert_eq!(code, Some(0));
    assert!(
        err.contains("bash: line 1: times: -z: invalid option\ntimes: usage: times"),
        "stderr was: {err}"
    );
    assert!(!err.contains("rubash:"), "stderr was: {err}");
}

/// B6-F3: at the top level of a -c string, `caller` argument validation
/// never runs — caller.def checks BASH_LINENO/BASH_SOURCE (then
/// FUNCNAME) emptiness before no_options / valid_number, so every
/// argument form is a silent status 1.
#[test]
fn b6_f3_caller_top_level_is_silent_for_all_argument_forms() {
    for form in [
        "caller -z",
        "caller abc",
        "caller -1",
        "caller 1x",
        "caller -- -z",
    ] {
        let (out, err, _code) = rubash_c(&format!("{form}; echo rc=$?"));
        assert_eq!(out, "rc=1\n", "form {form}");
        assert_eq!(err, "", "form {form} must be silent, stderr was: {err}");
    }
}

/// B6-F3: inside a function the argument model applies in full —
/// option scan (getopt names `-` + second char), the post-`--` operand
/// rule, the lone-dash operand, GNU valid_number grammar, and
/// sh_invalidnum's octal/hex message variants.
#[test]
fn b6_f3_caller_argument_model_inside_function() {
    // Diagnostics are folded into stdout with 2>&1 so one capture holds
    // the whole transcript in order.
    let (out, _err, _code) = rubash_c(
        "f() { caller -zx 2>&1; caller -- -z 2>&1; caller - 2>&1; caller ' 1'; echo ws=$?; \
         caller 0x10 2>&1; caller 08; echo oct=$?; }; f",
    );
    assert!(out.contains("caller: -z: invalid option"), "stdout: {out}");
    assert!(out.contains("caller: -z: invalid number"), "stdout: {out}");
    assert!(out.contains("caller: -: invalid number"), "stdout: {out}");
    assert!(
        out.contains("caller: 0x10: invalid hex number"),
        "stdout: {out}"
    );
    assert!(
        out.contains("ws=1"),
        "leading-whitespace number is a frame lookup: {out}"
    );
    assert!(
        out.contains("oct=1"),
        "`08` is decimal 8, an absent frame: {out}"
    );
}

/// B6-F3: in a script FILE the top level validates too — GNU's raw
/// funcname_a keeps an invisible "main" element (shell.c
/// open_shell_script), so the caller.def:108 emptiness check does not
/// silence argument validation there, even though the visible
/// FUNCNAME array is empty.
#[test]
fn b6_f3_caller_script_top_level_validates() {
    let (_out, err, _code) = rubash_file("caller abc\ncaller -z\ncaller 0; echo c0=$?\n");
    assert!(
        err.contains("caller: abc: invalid number"),
        "stderr was: {err}"
    );
    assert!(
        err.contains("caller: -z: invalid option"),
        "stderr was: {err}"
    );
}

/// B6-F3: `caller --help` is CHECK_HELPOPT (builtins/common.h:29) —
/// long help on stdout, status EX_USAGE (2).
#[test]
fn b6_f3_caller_help() {
    let (out, _err, _code) = rubash_c("f() { caller --help; echo rc=$?; }; f");
    assert!(out.contains("caller: caller [expr]"), "stdout was: {out}");
    assert!(out.contains("rc=2"), "stdout was: {out}");

    // CHECK_HELPOPT precedes even the array checks: top level works too.
    let (out, _err, _code) = rubash_c("caller --help; echo rc=$?");
    assert!(out.contains("caller: caller [expr]"), "stdout was: {out}");
    assert!(out.contains("rc=2"), "stdout was: {out}");
}

/// B6-F4: under `wait -n`, a pid that is not a current job takes
/// sh_badpid's "`N': not a pid or valid job spec" (builtins/wait.def
/// set_waitlist; builtins/common.c:234); plain `wait` keeps the
/// wait_for_single_pid text (jobs.c:2754).
#[test]
fn b6_f4_wait_next_non_job_pid_message() {
    let (_out, err, _code) = rubash_c("wait -n 999999; echo rc=$?");
    assert!(
        err.contains("wait: `999999': not a pid or valid job spec"),
        "stderr was: {err}"
    );

    let (_out, err, _code) = rubash_c("wait -n -p v 12345; echo rc=$?");
    assert!(
        err.contains("wait: `12345': not a pid or valid job spec"),
        "stderr was: {err}"
    );

    let (_out, err, _code) = rubash_c("wait 999999; echo rc=$?");
    assert!(
        err.contains("wait: pid 999999 is not a child of this shell"),
        "plain wait must keep the jobs.c text, stderr was: {err}"
    );
}
