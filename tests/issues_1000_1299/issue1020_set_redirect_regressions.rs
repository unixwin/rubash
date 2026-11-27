//! rubash#1020: `set` with arguments silently dropped redirections.
//!
//! GNU spec: execute_cmd.c:5584 `execute_builtin_or_function()` runs
//! `do_redirections` BEFORE the builtin runs (a failed redirect reports
//! EX_REDIRFAIL without running the builtin). The executor fast paths in
//! `execute_set_command` (`apply_simple_set_flags`,
//! `apply_set_positional_operands`, the `set --` branch) used to return
//! without ever applying the command's redirects — no file, no error,
//! rc=0 — while bare `set > file` worked.

use std::fs;
use std::process::Command;

fn run(dir: &std::path::Path, script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code(),
    )
}

fn fresh_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn set_positional_with_redirect_creates_file() {
    let dir = fresh_dir("rubash1020-positional");
    // GNU: rc=0, o1.txt created empty, positional params assigned.
    let (stdout, stderr, code) = run(&dir, "set -- -a -b > o1.txt; echo \"rc=$?\"; echo $1");
    assert_eq!(stdout, "rc=0\n-a\n", "{stderr}");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(fs::read_to_string(dir.join("o1.txt")).unwrap(), "");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn set_flag_with_redirect_creates_file_and_applies_flag() {
    let dir = fresh_dir("rubash1020-flag");
    // GNU: rc=0, file created; -e takes effect ($- contains e).
    let (stdout, stderr, code) = run(&dir, "set -e > of_e.txt; echo \"rc=$?\"; echo $-");
    assert!(stdout.starts_with("rc=0\n"), "{stdout}");
    assert!(stdout.contains('e'), "{stdout}");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(fs::read_to_string(dir.join("of_e.txt")).unwrap(), "");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn set_with_failing_redirect_reports_and_fails() {
    let dir = fresh_dir("rubash1020-failing");
    // GNU: "nodir/x.txt: No such file or directory" on stderr, rc=1.
    // (The script's trailing `echo` exits 0; the failing `set` status
    // shows in the captured rc line above.)
    let (stdout, stderr, code) = run(&dir, "set -- -b 2>nodir/x.txt; echo \"rc=$?\"");
    assert_eq!(stdout, "rc=1\n", "{stderr}");
    assert!(
        stderr.contains("nodir/x.txt") && stderr.contains("No such file or directory"),
        "{stderr}"
    );
    assert_eq!(code, Some(0));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn set_noclobber_flag_with_redirect_to_fresh_file() {
    let dir = fresh_dir("rubash1020-noclobber");
    // The -C flag fast path must still apply the redirect.
    let (stdout, stderr, code) = run(&dir, "set -C > fresh.txt; echo \"rc=$?\"");
    assert_eq!(stdout, "rc=0\n", "{stderr}");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(fs::read_to_string(dir.join("fresh.txt")).unwrap(), "");
    let _ = fs::remove_dir_all(&dir);
}
