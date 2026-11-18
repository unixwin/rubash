//! rubash#2697 — a signal-death notice for a member command killed
//! inside a foreground subshell renders the whole subshell text in
//! parens instead of the killed member command.
//!
//! GNU's rule (bash-5.3 jobs.c): the notice prints the dying
//! process's own job-table command (`p->command`, print_pipeline
//! jobs.c:2115 via pretty_print_job jobs.c:2172). Two different
//! shells print two different jobs:
//! - A member command killed inside a foreground subshell is
//!   reported by the SUBSHELL's own job table, where the member is
//!   a plain job whose command is the member's own printed text
//!   (execute_cmd.c:2777 line) — the BARE member text.
//! - When the subshell PROCESS itself dies (the `exec` form, or a
//!   background subshell job being killed), the PARENT's job table
//!   holds `make_command_string(command)` for that process
//!   (execute_cmd.c:701 make_child) — the WHOLE subshell text.
//!
//! rubash runs subshells flat, and `signal_notice_command_text`
//! (src/executor/job_builtins.rs) wraps the member text in parens
//! whenever a subshell is open — conflating the parent's view of a
//! subshell job with the inner shell's view of its member job.
//!
//! Expectations verified against GNU bash 5.3.20 (script files,
//! 2026-11-09). The pid and the line stamp are normalized out of
//! the notice assertions (the stamp is rubash#2769's face); the
//! command TEXT is asserted byte-for-byte.

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2697-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run(tag: &str, script: &str) -> (String, String) {
    let dir = tempdir(tag);
    std::fs::write(dir.join("probe.sh"), script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let _ = std::fs::remove_dir_all(&dir);
    (stdout, stderr)
}

/// The command text of the (single) signal-death notice in stderr:
/// the text after the signal description and its padding.
fn notice_text(stderr: &str, description: &str) -> String {
    let line = stderr
        .lines()
        .find(|line| line.contains(description))
        .unwrap_or_else(|| panic!("no notice with {description:?} in stderr: {stderr}"));
    let after = line
        .split_once(description)
        .unwrap_or_else(|| panic!("notice line lacks description: {line}"))
        .1;
    after.trim_start().to_string()
}

/// The deep-audit q15 shape: a foreground subshell whose `dd`
/// member is killed by SIGXFSZ. GNU names the member command
/// itself; the subshell survives and reports dd-rc=153.
#[test]
fn fg_subshell_member_death_names_the_member_command() {
    let (stdout, stderr) = run(
        "q15",
        "( ulimit -f 1; dd if=/dev/zero of=big bs=1024 count=4 2>/dev/null; echo dd-rc=$? )\n\
         echo after\n\
         rm -f big\n",
    );
    assert_eq!(stdout, "dd-rc=153\nafter\n", "stderr: {stderr}");
    assert_eq!(
        notice_text(&stderr, "File size limit exceeded"),
        "dd if=/dev/zero of=big bs=1024 count=4 2> /dev/null",
        "stderr: {stderr}"
    );
}

/// The named text tracks whichever member is actually running —
/// it is the member's own command text, not a fixed string.
#[test]
fn fg_subshell_member_death_names_whichever_member_is_running() {
    let (stdout, stderr) = run(
        "q15b",
        "( ulimit -f 1; dd if=/dev/zero of=big2 bs=512 count=16 2>/dev/null; echo dd-rc=$? )\n\
         echo after\n\
         rm -f big2\n",
    );
    assert_eq!(stdout, "dd-rc=153\nafter\n", "stderr: {stderr}");
    assert_eq!(
        notice_text(&stderr, "File size limit exceeded"),
        "dd if=/dev/zero of=big2 bs=512 count=16 2> /dev/null",
        "stderr: {stderr}"
    );
}

/// Control (the shape-split's other half): with `exec`, the
/// subshell process BECOMES dd, so the dying process is the
/// subshell job itself and GNU names the whole subshell text.
/// (rubash#1493's shape, asserted at text level here.)
#[test]
fn fg_subshell_exec_death_names_the_whole_subshell_text() {
    let (stdout, stderr) = run(
        "exec",
        "( ulimit -f 0; exec dd if=/dev/zero of=big bs=1024 count=100 2>/dev/null )\n\
         echo \"rc=$?\"\n\
         rm -f big\n",
    );
    assert_eq!(stdout, "rc=153\n", "stderr: {stderr}");
    assert_eq!(
        notice_text(&stderr, "File size limit exceeded"),
        "( ulimit -f 0; exec dd if=/dev/zero of=big bs=1024 count=100 2> /dev/null )",
        "stderr: {stderr}"
    );
}

/// Control: a background job that IS a subshell is the parent's
/// job; killing it names the whole subshell text in GNU too.
#[test]
fn bg_subshell_job_death_names_the_whole_subshell_text() {
    let (stdout, stderr) = run(
        "bgjob",
        "( dd if=/dev/zero of=/dev/null ) &\n\
         kill -KILL %1\n\
         sleep 0.3\n\
         echo done\n",
    );
    assert_eq!(stdout, "done\n", "stderr: {stderr}");
    assert_eq!(
        notice_text(&stderr, "Killed"),
        "( dd if=/dev/zero of=/dev/null )",
        "stderr: {stderr}"
    );
}

/// Control: no subshell at all — a plain foreground member death
/// names the member command (the long-standing shape).
#[test]
fn plain_member_death_names_the_member_command() {
    let (stdout, stderr) = run(
        "plain",
        "ulimit -f 1\n\
         dd if=/dev/zero of=big bs=1024 count=4 2>/dev/null\n\
         echo \"rc=$?\"\n\
         rm -f big\n",
    );
    assert_eq!(stdout, "rc=153\n", "stderr: {stderr}");
    assert_eq!(
        notice_text(&stderr, "File size limit exceeded"),
        "dd if=/dev/zero of=big bs=1024 count=4 2> /dev/null",
        "stderr: {stderr}"
    );
}
