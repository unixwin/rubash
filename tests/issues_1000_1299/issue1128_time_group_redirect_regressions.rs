//! rubash#1128 — `time` keyword output ignores an enclosing group's
//! redirection.
//!
//! GNU execute_cmd.c (time_command): the timing report is written to
//! the stderr in effect for the timed command's context — a group
//! redirection `{ time -p true; } 2>file` captures the three report
//! lines into the file, and nothing leaks to the shell's own stderr.
//!
//! Expectations verified against GNU bash 5.3.20 (script run in a
//! scratch directory): the file holds exactly the three `-p` report
//! lines (values are timing-dependent, the line shapes are not).

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1128-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn time_output_follows_enclosing_group_redirection() {
    let dir = tempdir("group");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("-c")
        .arg("{ time -p true; } 2>tf.txt; echo \"rc=$?\"")
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, "", "timing report leaked to shell stderr");
    let report = std::fs::read_to_string(dir.join("tf.txt")).expect("tf.txt written");
    let lines: Vec<&str> = report.lines().collect();
    assert_eq!(lines.len(), 3, "report: {report:?}");
    assert!(lines[0].starts_with("real "), "report: {report:?}");
    assert!(lines[1].starts_with("user "), "report: {report:?}");
    assert!(lines[2].starts_with("sys "), "report: {report:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
