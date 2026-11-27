//! rubash#1278 — `set -e`: timing stats dropped when errexit fires
//! on `time X` (GNU still prints them).
//!
//! GNU execute_cmd.c (time_command): the timing report belongs to
//! the timed command's completion, not to whatever errexit does
//! next — when `time false` fails under `set -e`, the report is
//! still written (to stderr) before the shell exits, in both the
//! default and the `-p` formats.
//!
//! Expectations verified against GNU bash 5.3.20 (`-c` strings): the
//! report VALUES are timing-dependent; the line shapes, the empty
//! stdout and rc=1 are the pin.

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
fn time_report_printed_when_errexit_fires() {
    let (stdout, stderr, code) = rubash("set -e\ntime false\necho lived");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("real\t0m") && stderr.contains("user\t0m") && stderr.contains("sys\t0m"),
        "timing report missing; stderr: {stderr:?}"
    );
    assert_eq!(code, Some(1));
}

#[test]
fn time_p_report_printed_when_errexit_fires() {
    let (stdout, stderr, code) = rubash("set -e\ntime -p false\necho lived");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("real 0.") && stderr.contains("user 0.") && stderr.contains("sys 0."),
        "timing report missing; stderr: {stderr:?}"
    );
    assert_eq!(code, Some(1));
}
