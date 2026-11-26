//! Issue rubash#1643: a bare `builtin` carrying only redirections
//! ignored them — `builtin > f` created nothing. GNU performs the
//! invocation's own redirections before (not) running a builtin:
//! do_redirections applies to the command node, so `builtin > f`
//! creates/truncates f, `builtin >> f` and `builtin 2> e` create their
//! files, and a failing redirection fails the invocation with
//! status 1. The bare-builtin arm now runs the same no-output redirect
//! helper the `:`/`true`/`false` arms use.
//!
//! Assertions follow the GNU bash 5.3.20 behavior measured with the
//! same scripts.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    #[allow(dead_code)]
    code: Option<i32>,
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1643-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn rubash_c(script: &str, dir: &Path) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

#[test]
fn issue1643_bare_builtin_output_redirect_creates_file() {
    let dir = tempdir("create");
    let run = rubash_c(
        "builtin > f; test -e f && echo f_exists || echo f_missing",
        &dir,
    );
    assert_eq!(run.stdout, "f_exists\n", "stderr: {}", run.stderr);
    assert_eq!(std::fs::read_to_string(dir.join("f")).unwrap(), "");
}

#[test]
fn issue1643_bare_builtin_append_redirect_creates_file() {
    let dir = tempdir("append");
    let run = rubash_c(
        "builtin >> f2; test -e f2 && echo f2_exists || echo f2_missing",
        &dir,
    );
    assert_eq!(run.stdout, "f2_exists\n", "stderr: {}", run.stderr);
}

#[test]
fn issue1643_bare_builtin_stderr_redirect_creates_file() {
    let dir = tempdir("stderr");
    let run = rubash_c(
        "builtin 2> e; test -e e && echo e_exists || echo e_missing",
        &dir,
    );
    assert_eq!(run.stdout, "e_exists\n", "stderr: {}", run.stderr);
}

#[test]
fn issue1643_bare_builtin_output_redirect_truncates_file() {
    let dir = tempdir("truncate");
    let run = rubash_c("echo OLD > f; builtin > f; wc -c < f", &dir);
    assert_eq!(run.stdout.trim(), "0", "stderr: {}", run.stderr);
    assert_eq!(std::fs::read_to_string(dir.join("f")).unwrap(), "");
}

#[test]
fn issue1643_bare_builtin_failed_output_redirect_status_1() {
    let dir = tempdir("badout");
    let run = rubash_c("builtin > /no-such-dir-xyz/f; echo rc_inner=$?", &dir);
    assert_eq!(run.stdout, "rc_inner=1\n", "stderr: {}", run.stderr);
}

#[test]
fn issue1643_bare_builtin_dashdash_output_redirect_creates_file() {
    let dir = tempdir("dashdash");
    let run = rubash_c(
        "builtin -- > f; test -e f && echo f_exists || echo f_missing",
        &dir,
    );
    assert_eq!(run.stdout, "f_exists\n", "stderr: {}", run.stderr);
}

#[test]
fn issue1643_bare_builtin_no_redirect_stays_success() {
    let dir = tempdir("plain");
    let run = rubash_c("builtin; echo rc_inner=$?", &dir);
    assert_eq!(run.stdout, "rc_inner=0\n", "stderr: {}", run.stderr);
}

#[test]
fn issue1643_builtin_with_name_still_runs_it() {
    // Control: `builtin echo` is not the bare form — the named builtin
    // runs and its own output redirection applies as before.
    let dir = tempdir("named");
    let run = rubash_c("builtin echo HI > fe; cat fe", &dir);
    assert_eq!(run.stdout, "HI\n", "stderr: {}", run.stderr);
}

#[test]
fn issue1643_bare_builtin_numbered_fd_redirect_known_gap() {
    let dir = tempdir("fd3");
    let run = rubash_c(
        "builtin 3>f3; test -e f3 && echo f3_exists || echo f3_missing",
        &dir,
    );
    assert_eq!(run.stdout, "f3_exists\n", "stderr: {}", run.stderr);
}
