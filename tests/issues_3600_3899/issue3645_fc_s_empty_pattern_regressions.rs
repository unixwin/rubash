//! rubash#3645 -- `fc -s =x` (empty-pattern substitution) was read as a
//! command name: the parser's `eq > 0` gate rejected a leading `=`, so
//! `fc -s =x` reported "fc: no command found" (rc 1).
//!
//! GNU model (bash 5.2.21, verified by oracle probe):
//! - builtins/fc.def (execute form): any argument containing `=` is a
//!   substitution -- `=x` has an empty pattern and `x` as the replacement.
//! - stringlib.c `strsub` with global=1: an empty pattern consumes one
//!   byte per match, so each byte of the command is replaced by one copy
//!   of the replacement (`true` -> `xxxx`, multibyte e-acute -> `xx`).
//! - When the replacement is empty too, nothing is ever appended and
//!   strsub returns the command unchanged (`fc -s =` is the identity).
//! - Substitutions apply in order, each on the previous result.

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3645-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run_rubash(script: &str, dir: &PathBuf) -> (String, String, i32) {
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

fn expect_reexec(stderr: &str, substituted: &str) {
    let mut lines = stderr.lines();
    assert_eq!(lines.next(), Some(substituted), "stderr: {stderr}");
    let rest = lines.next().unwrap_or("");
    assert!(
        rest.ends_with(": command not found"),
        "expected a command-not-found diagnostic, stderr: {stderr}"
    );
}

#[test]
fn fc_s_empty_pattern_replaces_each_byte() {
    let dir = tempdir("empty-pat");
    let (stdout, stderr, rc) = run_rubash("history -s 'true'\nfc -s =x\n", &dir);
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert_eq!(rc, 127, "stderr: {stderr}");
    expect_reexec(&stderr, "xxxx");
}

#[test]
fn fc_s_empty_pattern_keeps_spaces_as_bytes() {
    let dir = tempdir("spaces");
    let (stdout, stderr, rc) = run_rubash("history -s 'echo hi'\nfc -s =x\n", &dir);
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert_eq!(rc, 127, "stderr: {stderr}");
    expect_reexec(&stderr, "xxxxxxx");
}

#[test]
fn fc_s_empty_pattern_empty_rep_is_identity() {
    let dir = tempdir("identity");
    let (stdout, stderr, rc) = run_rubash("history -s 'true'\nfc -s =\n", &dir);
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert_eq!(rc, 0, "stderr: {stderr}");
    assert_eq!(stderr, "true\n", "stderr: {stderr}");
}

#[test]
fn fc_s_empty_pattern_with_spec() {
    let dir = tempdir("spec");
    let (stdout, stderr, rc) = run_rubash(
        "history -s 'true'\nhistory -s 'echo hi'\nfc -s =x true\n",
        &dir,
    );
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert_eq!(rc, 127, "stderr: {stderr}");
    expect_reexec(&stderr, "xxxx");
}

#[test]
fn fc_s_empty_pattern_multibyte_is_per_byte() {
    let dir = tempdir("multibyte");
    let (stdout, stderr, rc) = run_rubash("history -s 'é'\nfc -s =x\n", &dir);
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert_eq!(rc, 127, "stderr: {stderr}");
    expect_reexec(&stderr, "xx");
}

#[test]
fn fc_s_empty_pattern_two_subs_apply_in_order() {
    let dir = tempdir("two-subs");
    let (stdout, stderr, rc) = run_rubash("history -s 'echo ab'\nfc -s =x =y\n", &dir);
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert_eq!(rc, 127, "stderr: {stderr}");
    expect_reexec(&stderr, "yyyyyyy");
}

#[test]
fn fc_s_nonempty_pattern_still_works() {
    let dir = tempdir("nonempty");
    let (stdout, stderr, rc) = run_rubash("history -s 'echo aa ab ac'\nfc -s a=x\n", &dir);
    assert_eq!(stdout, "xx xb xc\n", "stderr: {stderr}");
    assert_eq!(rc, 0, "stderr: {stderr}");
    assert_eq!(stderr, "echo xx xb xc\n", "stderr: {stderr}");
}
