//! rubash#1156 — Bare `wait -p` panics the shell (SIGABRT rc=134).
//!
//! GNU builtins/wait.def: `-p` requires an argument; a bare
//! `wait -p` is a usage error (diagnostic + usage line, rc=2) and the
//! shell CONTINUES. It must never panic, whatever the option mix.
//!
//! Expectations verified against GNU bash 5.3.20 (`-c` strings). The
//! `wait -p var $!` form stores a live pid whose value is
//! kernel-assigned; that line is matched on shape, the rest is exact.

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
fn bare_wait_p_is_usage_error_not_panic() {
    let (stdout, stderr, code) = rubash("wait -p; echo \"rc=$?\"\necho survived");
    assert_eq!(stdout, "rc=2\nsurvived\n");
    assert!(
        stderr.contains("wait: -p: option requires an argument"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0), "the shell itself must exit normally");
}

#[test]
fn wait_p_option_matrix_matches_gnu() {
    let (stdout, stderr, code) = rubash(
        "wait -np; echo \"rc=$?\"\n\
         wait -p; echo \"rc=$?\"\n\
         sleep 0.05 & wait -p v $!; echo \"rc=$? v=$v\"\n\
         wait -x; echo \"rc=$?\"\n\
         wait -n -p; echo \"rc=$?\"",
    );
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 5, "stdout: {stdout}");
    assert_eq!(lines[0], "rc=2");
    assert_eq!(lines[1], "rc=2");
    assert!(
        lines[2].starts_with("rc=0 v=") && lines[2][7..].chars().all(|c| c.is_ascii_digit()),
        "wait -p var must store the job pid; line: {}",
        lines[2]
    );
    assert_eq!(lines[3], "rc=2");
    assert_eq!(lines[4], "rc=2");
    assert!(
        stderr.contains("wait: -x: invalid option"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}
