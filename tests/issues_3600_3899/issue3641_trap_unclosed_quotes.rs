//! Unclosed matched pairs (quotes / `$(...)` / etc.) in trap actions
//! (rubash#3641; GNU bash 5.3.20 oracle-verified).
//!
//! GNU's `parse_and_execute` consumes each trap action one complete unit
//! at a time: complete units before the unclosed pair's outermost
//! pending construct execute, and the unit with the unclosed quote
//! fails with `unexpected EOF while looking for matching '...'` instead
//! of silently executing the folded word text.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn rubash_bin() -> String {
    env!("CARGO_BIN_EXE_rubash").to_string()
}

fn run_script(tag: &str, body: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("issue3641-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    let path: PathBuf = dir.join("case.sh");
    fs::write(&path, body).expect("write script");
    let output = Command::new(rubash_bin())
        .arg(&path)
        .env("LC_ALL", "C.UTF-8")
        .env_remove("BASH_ENV")
        .output()
        .expect("run rubash");
    let normalize = |bytes: &[u8]| {
        let mut text = String::from_utf8_lossy(bytes).into_owned();
        let raw = path.to_str().expect("utf8 path");
        let fwd = raw.replace('\\', "/");
        let mut needles = vec![raw.to_string(), fwd.clone()];
        if fwd.len() > 2 && fwd.as_bytes()[1] == b':' && fwd.as_bytes()[0].is_ascii_alphabetic() {
            needles.push(format!("/{}{}", fwd[..1].to_lowercase(), &fwd[2..]));
        }
        for needle in needles {
            if !needle.is_empty() {
                text = text.replace(&needle, "S");
            }
        }
        text
    };
    let result = (
        normalize(&output.stdout),
        normalize(&output.stderr),
        output.status.code(),
    );
    let _ = fs::remove_dir_all(&dir);
    result
}

#[test]
fn issue3641_exit_trap_unclosed_quote_variants_and_control() {
    // Single-line unclosed quote
    let (stdout, stderr, code) = run_script("exit-single", "trap \"echo 'ab\" EXIT\necho main\n");
    assert_eq!(stdout, "main\n");
    assert_eq!(code, Some(0));
    assert!(
        stderr.contains("S: exit trap: line 1: unexpected EOF while looking for matching `''"),
        "exit single-line diag: {stderr:?}"
    );

    // Multi-line prefix: complete line before the unclosed quote executes
    let (stdout, stderr, code) = run_script(
        "exit-multiline",
        "trap $'echo before\\necho \\'ab' EXIT\necho main\n",
    );
    assert_eq!(stdout, "main\nbefore\n");
    assert_eq!(code, Some(0));
    assert!(
        stderr.contains("S: exit trap: line 2: unexpected EOF while looking for matching `''"),
        "exit multiline diag: {stderr:?}"
    );

    // Compound-with-quote: unclosed quote inside unclosed compound reports the quote
    let (stdout, stderr, code) = run_script(
        "exit-compound",
        "trap $'if true; then\\necho \\'ab' EXIT\necho main\n",
    );
    assert_eq!(stdout, "main\n");
    assert_eq!(code, Some(0));
    assert!(
        stderr.contains("unexpected EOF while looking for matching `''"),
        "exit compound-with-quote diag: {stderr:?}"
    );

    // Valid control
    let (stdout, stderr, code) = run_script("exit-valid", "trap \"echo 'ok'\" EXIT\necho main\n");
    assert_eq!(stdout, "main\nok\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3641_err_trap_unclosed_quote_variants_and_control() {
    // Single-line unclosed quote
    let (stdout, stderr, code) =
        run_script("err-single", "trap \"echo 'ab\" ERR\nfalse\necho after\n");
    assert_eq!(stdout, "after\n");
    assert_eq!(code, Some(0));
    assert!(
        stderr.contains("S: error trap: line 2: unexpected EOF while looking for matching `''"),
        "err single-line diag: {stderr:?}"
    );

    // Multi-line prefix
    let (stdout, stderr, code) = run_script(
        "err-multiline",
        "trap $'echo err-before\\necho \\'ab' ERR\nfalse\necho after\n",
    );
    assert_eq!(stdout, "err-before\nafter\n");
    assert_eq!(code, Some(0));
    assert!(
        stderr.contains("S: error trap: line 3: unexpected EOF while looking for matching `''"),
        "err multiline diag: {stderr:?}"
    );

    // Compound-with-quote
    let (stdout, stderr, code) = run_script(
        "err-compound",
        "trap $'if true; then\\necho \\'ab' ERR\nfalse\necho after\n",
    );
    assert_eq!(stdout, "after\n");
    assert_eq!(code, Some(0));
    assert!(
        stderr.contains("S: error trap: line 3: unexpected EOF while looking for matching `''"),
        "err compound-with-quote diag: {stderr:?}"
    );

    // Valid control
    let (stdout, stderr, code) = run_script(
        "err-valid",
        "trap \"echo 'err-ok'\" ERR\nfalse\necho after\n",
    );
    assert_eq!(stdout, "err-ok\nafter\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3641_debug_trap_unclosed_quote_and_extdebug() {
    // Single-line unclosed quote
    let (stdout, stderr, code) = run_script("debug-single", "trap \"echo 'ab\" DEBUG\necho body\n");
    assert_eq!(stdout, "body\n");
    assert_eq!(code, Some(0));
    assert!(
        stderr.contains("S: debug trap: line 2: unexpected EOF while looking for matching `''"),
        "debug single-line diag: {stderr:?}"
    );

    // Multi-line prefix
    let (stdout, stderr, code) = run_script(
        "debug-multiline",
        "trap $'echo dbg-before\\necho \\'ab' DEBUG\necho body\n",
    );
    assert_eq!(stdout, "dbg-before\nbody\n");
    assert_eq!(code, Some(0));
    assert!(
        stderr.contains("S: debug trap: line 3: unexpected EOF while looking for matching `''"),
        "debug multiline diag: {stderr:?}"
    );

    // Compound-with-quote
    let (stdout, stderr, code) = run_script(
        "debug-compound",
        "trap $'if true; then\\necho \\'ab' DEBUG\necho body\n",
    );
    assert_eq!(stdout, "body\n");
    assert_eq!(code, Some(0));
    assert!(
        stderr.contains("S: debug trap: line 3: unexpected EOF while looking for matching `''"),
        "debug compound-with-quote diag: {stderr:?}"
    );

    // extdebug mode: unclosed quote sets nonzero trap status (1), which skips the command
    let (stdout, stderr, code) = run_script(
        "debug-extdebug",
        "shopt -s extdebug\ntrap \"echo 'ab\" DEBUG\necho should-skip\n",
    );
    assert_eq!(stdout, "");
    assert_eq!(code, Some(0));
    assert!(
        stderr.contains("S: debug trap: line 3: unexpected EOF while looking for matching `''"),
        "debug extdebug diag: {stderr:?}"
    );

    // Valid control
    let (stdout, stderr, code) =
        run_script("debug-valid", "trap \"echo 'dbg-ok'\" DEBUG\necho body\n");
    assert_eq!(stdout, "dbg-ok\nbody\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3641_return_trap_unclosed_quote_variants_and_control() {
    // Single-line unclosed quote
    let (stdout, stderr, code) = run_script(
        "return-single",
        "set -T\ntrap \"echo 'ab\" RETURN\nf() { :; }\nf\necho after\n",
    );
    assert_eq!(stdout, "after\n");
    assert_eq!(code, Some(0));
    assert!(
        stderr.contains("S: return trap: line 3: unexpected EOF while looking for matching `''"),
        "return single-line diag: {stderr:?}"
    );

    // Multi-line prefix
    let (stdout, stderr, code) = run_script(
        "return-multiline",
        "set -T\ntrap $'echo ret-before\\necho \\'ab' RETURN\nf() { :; }\nf\necho after\n",
    );
    assert_eq!(stdout, "ret-before\nafter\n");
    assert_eq!(code, Some(0));
    assert!(
        stderr.contains("S: return trap: line 4: unexpected EOF while looking for matching `''"),
        "return multiline diag: {stderr:?}"
    );

    // Compound-with-quote
    let (stdout, stderr, code) = run_script(
        "return-compound",
        "set -T\ntrap $'if true; then\\necho \\'ab' RETURN\nf() { :; }\nf\necho after\n",
    );
    assert_eq!(stdout, "after\n");
    assert_eq!(code, Some(0));
    assert!(
        stderr.contains("S: return trap: line 4: unexpected EOF while looking for matching `''"),
        "return compound-with-quote diag: {stderr:?}"
    );

    // Valid control
    let (stdout, stderr, code) = run_script(
        "return-valid",
        "set -T\ntrap \"echo 'ret-ok'\" RETURN\nf() { :; }\nf\necho after\n",
    );
    assert_eq!(stdout, "ret-ok\nafter\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3641_signal_trap_unclosed_quote_variants_and_control() {
    // Single-line unclosed quote
    let (stdout, stderr, code) = run_script(
        "signal-single",
        "trap \"echo 'ab\" USR1\nkill -USR1 $$\necho after\n",
    );
    assert_eq!(stdout, "after\n");
    assert_eq!(code, Some(0));
    assert!(
        stderr.contains("unexpected EOF while looking for matching `''"),
        "signal single-line diag: {stderr:?}"
    );

    // Multi-line prefix
    let (stdout, stderr, code) = run_script(
        "signal-multiline",
        "trap $'echo sig-before\\necho \\'ab' USR1\nkill -USR1 $$\necho after\n",
    );
    assert_eq!(stdout, "sig-before\nafter\n");
    assert_eq!(code, Some(0));
    assert!(
        stderr.contains("unexpected EOF while looking for matching `''"),
        "signal multiline diag: {stderr:?}"
    );

    // Compound-with-quote
    let (stdout, stderr, code) = run_script(
        "signal-compound",
        "trap $'if true; then\\necho \\'ab' USR1\nkill -USR1 $$\necho after\n",
    );
    assert_eq!(stdout, "after\n");
    assert_eq!(code, Some(0));
    assert!(
        stderr.contains("unexpected EOF while looking for matching `''"),
        "signal compound-with-quote diag: {stderr:?}"
    );

    // Valid control
    let (stdout, stderr, code) = run_script(
        "signal-valid",
        "trap \"echo 'sig-ok'\" USR1\nkill -USR1 $$\necho after\n",
    );
    assert_eq!(stdout, "sig-ok\nafter\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
