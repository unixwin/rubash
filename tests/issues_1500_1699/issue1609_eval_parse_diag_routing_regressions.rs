//! rubash#1609 — Parse diagnostics produced by `eval` bypassed the
//! caller's `2>&1` redirection and landed on the shell's real stderr.
//! GNU parser_error (error.c) writes to fd 2 as currently bound: a
//! builtin's redirections are bound before parse_and_execute reads the
//! string, so the diagnostic follows them (residual of #1114; the
//! bf-w3-a pin lives in `issue1114_eval_extglob_gate_regressions.rs`
//! and must pass once both land).
//!
//! Expectations verified against GNU bash 5.3.20: the syntax-error
//! wording is matched on its stable core (the line prefix carries the
//! binary's own path), the rc markers are byte-exact.

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
fn eval_parse_diagnostic_follows_caller_2_to_1() {
    let (stdout, stderr, code) = rubash(
        "v=\"abc123\"\n\
         eval 'case $v in +([a-z])+([0-9])) echo m1;; *) echo no;; esac' 2>&1; \
         echo \"pre-extglob-rc=$?\"\n\
         shopt -s extglob\n\
         case $v in +([a-z])+([0-9])) echo m2;; *) echo no2;; esac",
    );
    assert!(
        stdout.contains("syntax error near unexpected token `('"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("pre-extglob-rc=2\n"), "stdout: {stdout}");
    assert!(stdout.ends_with("m2\n"), "stdout: {stdout}");
    // `m1` may appear inside the offending-line echo; it must never
    // appear as an executed output line.
    assert!(!stdout.lines().any(|line| line == "m1"), "stdout: {stdout}");
    assert!(
        !stderr.contains("syntax error"),
        "diagnostic leaked to real stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn eval_parse_diagnostic_follows_caller_2_to_devnull() {
    let (stdout, stderr, code) =
        rubash("eval 'case x in +([a])) echo;; esac' 2>/dev/null; echo rc=$?");
    assert_eq!(stdout, "rc=2\n", "stdout: {stdout}");
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn eval_parse_diagnostic_follows_exec_fd2_binding() {
    // `exec 2>cap` rebinds the shell's fd 2 before the eval runs; GNU
    // writes the parse diagnostic into cap, not to the real stderr.
    let dir = std::env::temp_dir().join(format!("rubash1609-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("-c")
        .arg("exec 2>cap.txt\neval 'case x in +([a])) echo;; esac'\necho rc=$?")
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let cap = std::fs::read_to_string(dir.join("cap.txt")).expect("read cap");
    assert_eq!(stdout, "rc=2\n", "stdout: {stdout}");
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert!(
        cap.contains("syntax error near unexpected token `('"),
        "cap: {cap}"
    );
    assert_eq!(output.status.code(), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}
