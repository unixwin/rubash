//! Issue rubash#1612: under a pty (job control genuinely active),
//! `jobs` must observe a stop the shell's own `kill` sent — GNU's
//! job-control shell processes child state at every command boundary
//! (jobs.c sigchld_handler -> waitchld, with notify points at each
//! parse.y read_command), so the very next `jobs` already lists the
//! job Stopped. This is the mirror of #1607 (non-interactive fast
//! path publishing too early): on the terminal path rubash never
//! published at all.
//!
//! The pty form needs util-linux `script`; where it is unavailable
//! the test returns without asserting (the equivalent non-terminal
//! observation faces are pinned by the #1607 regression file).

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1612-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn jobs_observes_stopped_under_pty() {
    if Command::new("script").arg("--version").output().is_err() {
        return;
    }
    let dir = tempdir("pty");
    std::fs::write(
        dir.join("jobs.sh"),
        "set -m\nsleep 30 &\nkill -STOP %1\njobs\nkill -9 %1 2>/dev/null\nwait 2>/dev/null\n",
    )
    .expect("write jobs.sh");
    let rubash = env!("CARGO_BIN_EXE_rubash");
    let command = format!("{rubash} jobs.sh");
    let output = Command::new("script")
        .arg("-qec")
        .arg(&command)
        .arg("/dev/null")
        .current_dir(&dir)
        .output()
        .expect("run script");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let job1 = stdout
        .lines()
        .find(|line| line.starts_with("[1]"))
        .unwrap_or_else(|| panic!("no [1] line under pty: {stdout:?}"))
        .to_string();
    assert!(
        job1.contains("Stopped"),
        "pty jobs never observed the stop: {job1:?} (full: {stdout:?})"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
