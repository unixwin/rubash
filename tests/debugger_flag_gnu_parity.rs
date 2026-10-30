//! GNU `--debugger` parity regressions (shell.c:257 / :1547 start_debugger).
//!
//! Before this change rubash mapped BOTH `--debug` and `--debugger` onto
//! "set extdebug" — under GNU, `--debugger` sets debugging_mode only and
//! startup runs the bashdb starter file (`bashdb-main.inc`); a missing
//! starter warns internally and clears the mode (shell.c:1559-1561), and
//! the flag alone NEVER enables extdebug — bashdb's own init does.
//!
//! Oracle (WSL GNU bash 5.3.20, 2026-10-23, no bashdb installed):
//!
//! ```text
//! $ bash --debugger -c 'shopt -p extdebug; echo AFTER'
//! bash: /usr/local/share/bashdb/bashdb-main.inc: No such file or directory
//! bash: warning: cannot start debugger; debugging mode disabled
//! shopt -u extdebug
//! AFTER
//! ```
//!
//! GNU bakes ONE configure-time starter path; rubash cannot know the host
//! bash's prefix, so the search is `BASHDB_START_FILE` (documented seam for
//! Termux / custom --prefix hosts) then the two conventional locations.
//! The bare interactive shell has no start_debugger call site at all
//! (shell.c:795's `reading_shell_script || interactive_shell == 0` gate) —
//! structural, not testable without a pty.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[derive(Debug)]
struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-dbgpar-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn spawn(args: &[&str], envs: &[(&str, &str)], stdin: &[u8]) -> Run {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("BASHDB_START_FILE")
        .env_remove("BASH_ENV")
        .env_remove("__RUBASH_STARTUP_FILES_DONE")
        .env_remove("__RUBASH_DEBUGGER_MODE")
        .env_remove("__RUBASH_DEBUGGER_ATTEMPTED")
        .stdin(Stdio::piped());
    for (key, value) in envs {
        command.env(key, value);
    }
    let mut child = command.spawn().expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(stdin)
        .expect("write stdin");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

/// GNU transcript shape: miss on the starter file warns, disables the mode,
/// and the -c string still runs with extdebug OFF (the flag never set it).
#[test]
fn debugger_flag_miss_warns_and_disables() {
    let run = spawn(
        &["--debugger", "-c", "shopt -p extdebug; echo AFTER"],
        &[],
        b"",
    );
    assert_eq!(run.code, Some(0), "rc; stderr:\n{}", run.stderr);
    assert!(
        run.stderr
            .contains("warning: cannot start debugger; debugging mode disabled"),
        "internal warning; stderr:\n{}",
        run.stderr
    );
    assert!(
        run.stderr.contains("No such file or directory"),
        "open-failure diagnostic; stderr:\n{}",
        run.stderr
    );
    assert!(
        run.stdout.contains("shopt -u extdebug"),
        "extdebug must stay OFF; stdout:\n{}",
        run.stdout
    );
    assert!(
        run.stdout.contains("AFTER"),
        "script ran; stdout:\n{}",
        run.stdout
    );
}

/// The BASHDB_START_FILE seam reports the exact path it tried, GNU's
/// force_execute_file shape.
#[test]
fn debugger_flag_miss_names_the_seam_path() {
    let dir = scratch("miss-path");
    let missing = dir.join("nope.inc");
    let missing = missing.to_string_lossy().replace('\\', "/");
    let run = spawn(
        &["--debugger", "-c", "echo AFTER"],
        &[("BASHDB_START_FILE", missing.as_str())],
        b"",
    );
    assert!(
        run.stderr.contains(&missing) && run.stderr.contains("No such file or directory"),
        "tried path in diagnostic; stderr:\n{}",
        run.stderr
    );
}

/// A found starter file is sourced before the -c string; its shopt calls
/// take effect (this is how real bashdb arms extdebug).
#[test]
fn debugger_flag_sources_found_starter_before_command() {
    let dir = scratch("found");
    let starter = dir.join("stub-main.inc");
    std::fs::write(&starter, "echo BASHDB_LOADED\nshopt -s extdebug\n").expect("write starter");
    let starter = starter.to_string_lossy().replace('\\', "/");
    let run = spawn(
        &["--debugger", "-c", "shopt -p extdebug; echo AFTER"],
        &[("BASHDB_START_FILE", starter.as_str())],
        b"",
    );
    assert_eq!(run.code, Some(0), "rc; stderr:\n{}", run.stderr);
    let loaded = run.stdout.find("BASHDB_LOADED").expect("starter ran");
    let after = run.stdout.find("AFTER").expect("command ran");
    assert!(loaded < after, "starter precedes the -c string");
    assert!(
        run.stdout.contains("shopt -s extdebug"),
        "starter's shopt took effect; stdout:\n{}",
        run.stdout
    );
    assert!(
        !run.stderr.contains("cannot start debugger"),
        "no miss warning; stderr:\n{}",
        run.stderr
    );
}

/// Without the flag nothing warns and extdebug stays off.
#[test]
fn no_debugger_flag_no_warning() {
    let run = spawn(&["-c", "shopt -p extdebug; echo AFTER"], &[], b"");
    assert_eq!(run.code, Some(0), "rc; stderr:\n{}", run.stderr);
    assert!(
        !run.stderr.contains("debugger"),
        "silent without the flag; stderr:\n{}",
        run.stderr
    );
    assert!(run.stdout.contains("shopt -u extdebug"));
}

/// `--debug` is the niu extension (documented in this PR): GNU's shell.c:255
/// `debugging` variable is write-only — no bash code reads it, so the name
/// carries no observable GNU behavior to conflict with. niu arms extdebug
/// from the first command (the DEBUG-trap substrate; #497's native debugger
/// upgrades this meaning in place). No bashdb attempt, no warning.
#[test]
fn debug_flag_arms_extdebug() {
    let run = spawn(
        &["--debug", "-c", "shopt -p extdebug; echo AFTER"],
        &[],
        b"",
    );
    assert_eq!(
        run.code,
        Some(0),
        "rc; stderr:\n{} stdout:\n{}",
        run.stderr,
        run.stdout
    );
    assert!(
        run.stdout.contains("shopt -s extdebug") && run.stdout.contains("AFTER"),
        "extdebug ON and the command ran; stdout:\n{}",
        run.stdout
    );
    assert!(
        !run.stderr.contains("cannot start debugger"),
        "no bashdb attempt; stderr:\n{}",
        run.stderr
    );
}

/// The stdin script reader is one of GNU's start_debugger sites too
/// (shell.c:795 covers reading_shell_script).
#[test]
fn debugger_flag_covers_stdin_scripts() {
    let run = spawn(&["--debugger"], &[], b"echo AFTER\n");
    assert_eq!(run.code, Some(0), "rc; stderr:\n{}", run.stderr);
    assert!(
        run.stderr
            .contains("warning: cannot start debugger; debugging mode disabled"),
        "stdin reader attempts the starter; stderr:\n{}",
        run.stderr
    );
    assert!(run.stdout.contains("AFTER"));
}
