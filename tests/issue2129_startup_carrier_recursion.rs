//! Issue rubash#2129 (P0) / rubash#2133 regressions: the startup-shape
//! carriers (`__RUBASH_INTERACTIVE`, `__RUBASH_LOGIN_SHELL`, and the
//! intra-process startup-carrier family `__RUBASH_STARTUP_FILES_DONE`,
//! `__RUBASH_SOURCED_RC`, `__RUBASH_NO_PROFILE`, `__RUBASH_NO_RC`) are a
//! parent session's private bookkeeping, never an inheritable
//! invocation shape.
//!
//! The defect (Android/Termux field report, fzf --bash integration):
//! an interactive shell exports the shape carriers to every child; a
//! `-c` child that merely INHERITED both markers adopted the parent's
//! interactive-login shape, re-sourced the startup chain, and — with
//! an rc/profile line that respawns the shell (fzf's
//! `__fzf_orig_completion < <(complete -p ...)`) — recursed without
//! bound, one blocked generation per ~0.15s. GNU decides startup
//! behavior from the invocation itself (shell.c:540-549 interactive
//! only for `-i` or the no-operand tty pair; shell.c:497-503
//! LOGIN_SHELL only from `-l`/`--login`/argv[0] `-`), so the fix keys
//! every startup-shape decision on the process's OWN argv (the OWN_*
//! statics in src/main.rs), gives respawned internal children a hard
//! no-startup door, converts the run-once guard to a process-local
//! static (#2133), and strips the carrier family consume-on-read at
//! process entry so only this process's own argv can arm the
//! --noprofile/--norc suppressors again.
//!
//! Every probe counts startup-file executions in a counter file (one
//! line per sourcing) and is watchdog-bounded: a regression that
//! recurses fails by deadline + counter, never by hanging the suite.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const WATCHDOG: Duration = Duration::from_secs(20);

#[derive(Debug)]
struct Run {
    stdout: String,
    code: Option<i32>,
    timed_out: bool,
}

fn home(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i2129-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch home");
    dir
}

fn write_file(dir: &Path, name: &str, contents: &str) {
    std::fs::write(dir.join(name), contents).expect("write startup file");
}

/// A home whose startup files each append one tag line to `count`.
fn counting_home(tag: &str) -> PathBuf {
    let dir = home(tag);
    let count = dir.join("count").to_string_lossy().replace('\\', "/");
    write_file(
        &dir,
        ".bash_profile",
        &format!("echo profile >> \"{count}\"\n"),
    );
    write_file(&dir, ".bashrc", &format!("echo rc >> \"{count}\"\n"));
    write_file(
        &dir,
        ".bash_logout",
        &format!("echo logout >> \"{count}\"\n"),
    );
    write_file(&dir, "count", "");
    dir
}

fn tag_count(dir: &Path, tag: &str) -> usize {
    std::fs::read_to_string(dir.join("count"))
        .unwrap_or_default()
        .lines()
        .filter(|line| *line == tag)
        .count()
}

fn bin() -> String {
    env!("CARGO_BIN_EXE_rubash").replace('\\', "/")
}

/// Spawn the binary with a scrubbed environment (the whole carrier
/// family removed unless the probe re-adds it), output captured to
/// files so a recursing child can never fill a pipe and wedge the
/// watchdog, and a hard deadline after which the child is killed.
fn spawn(args: &[&str], home_dir: &Path, extra_env: &[(&str, &str)], stdin: &[u8]) -> Run {
    let scratch = home("io");
    let out_path = scratch.join("stdout");
    let in_path = scratch.join("stdin");
    std::fs::write(&in_path, stdin).expect("write stdin file");
    let stdout_file = std::fs::File::create(&out_path).expect("stdout file");
    let stdin_file = std::fs::File::open(&in_path).expect("stdin file");
    let mut command = Command::new(bin());
    command
        .args(args)
        .env("HOME", home_dir.to_string_lossy().replace('\\', "/"))
        .env("USERPROFILE", home_dir)
        .env_remove("HOMEDRIVE")
        .env_remove("HOMEPATH")
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .env_remove("NIU_INVOKED_AS")
        .env_remove("BASH_ARGV0")
        .env_remove("__RUBASH_INTERACTIVE")
        .env_remove("__RUBASH_LOGIN_SHELL")
        .env_remove("__RUBASH_STARTUP_FILES_DONE")
        .env_remove("__RUBASH_SOURCED_RC")
        .env_remove("__RUBASH_NO_PROFILE")
        .env_remove("__RUBASH_NO_RC")
        .env_remove("__RUBASH_SHELL_PID")
        .env_remove("__RUBASH_COPROC_CHILD")
        .stdin(Stdio::from(stdin_file))
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::null());
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let mut child = command.spawn().expect("spawn rubash");
    let deadline = Instant::now() + WATCHDOG;
    let code = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break Some(status.code().unwrap_or(-1)),
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            None => std::thread::sleep(Duration::from_millis(25)),
        }
    };
    let stdout = std::fs::read_to_string(&out_path).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&scratch);
    Run {
        stdout,
        code,
        timed_out: code.is_none(),
    }
}

const BOTH_MARKERS: &[(&str, &str)] =
    &[("__RUBASH_INTERACTIVE", "1"), ("__RUBASH_LOGIN_SHELL", "1")];

// ---------------------------------------------------------------------------
// Marker adoption: an inherited shape is not an invocation shape
// ---------------------------------------------------------------------------

/// Both markers inherited, plain `-c`: GNU runs no startup branch but
/// $BASH_ENV for this shape — profile/rc/logout counts stay zero and
/// the process simply completes (pre-fix it adopted the login branch).
#[test]
fn both_markers_inherited_c_runs_no_startup() {
    let dir = counting_home("both");
    let run = spawn(&["-c", "true"], &dir, BOTH_MARKERS, b"");
    assert!(!run.timed_out, "must terminate: {run:?}");
    assert_eq!(tag_count(&dir, "profile"), 0);
    assert_eq!(tag_count(&dir, "rc"), 0);
    assert_eq!(tag_count(&dir, "logout"), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Each marker alone is likewise inert for a `-c` child.
#[test]
fn single_markers_inherited_c_runs_no_startup() {
    for (tag, env) in [
        ("int-only", &[("__RUBASH_INTERACTIVE", "1")][..]),
        ("login-only", &[("__RUBASH_LOGIN_SHELL", "1")][..]),
    ] {
        let dir = counting_home(tag);
        let run = spawn(&["-c", "true"], &dir, env, b"");
        assert!(!run.timed_out, "{tag} must terminate");
        assert_eq!(tag_count(&dir, "profile"), 0, "{tag}");
        assert_eq!(tag_count(&dir, "rc"), 0, "{tag}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// The P0 minimal repro itself (fzf's completion procsub body as a
/// bare `-c` string under both inherited markers) terminates instead
/// of recursing, and sources nothing.
#[test]
fn p0_repro_terminates_and_sources_nothing() {
    let dir = counting_home("p0");
    let run = spawn(
        &[
            "-c",
            "complete -p $d_cmds $a_cmds $v_cmds unalias kill ssh 2> /dev/null",
        ],
        &dir,
        BOTH_MARKERS,
        b"",
    );
    assert!(!run.timed_out, "P0 repro must terminate: {run:?}");
    assert_eq!(tag_count(&dir, "profile"), 0);
    assert_eq!(tag_count(&dir, "rc"), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Own invocation shape still earns its startup branch
// ---------------------------------------------------------------------------

#[cfg(not(windows))]
#[test]
fn own_login_sources_profile_once() {
    let dir = counting_home("own-lc");
    let run = spawn(&["-lc", "true"], &dir, &[], b"");
    assert!(!run.timed_out);
    assert_eq!(tag_count(&dir, "profile"), 1);
    assert_eq!(tag_count(&dir, "rc"), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn own_interactive_sources_rc_once() {
    let dir = counting_home("own-ic");
    let run = spawn(&["-ic", "true"], &dir, &[], b"");
    assert!(!run.timed_out);
    assert_eq!(tag_count(&dir, "rc"), 1);
    assert_eq!(tag_count(&dir, "profile"), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// #2133: the run-once guard is process-local
// ---------------------------------------------------------------------------

/// A genuine nested login shell runs its OWN profile chain: the
/// parent's "startup done" state must not cross the spawn boundary
/// (pre-fix the exported DONE carrier suppressed the inner chain).
#[cfg(not(windows))]
#[test]
fn nested_login_sources_profile_twice() {
    let dir = counting_home("nested");
    let inner = format!("{} -lc true", bin());
    let run = spawn(&["-lc", &inner], &dir, &[], b"");
    assert!(!run.timed_out, "nested login must terminate: {run:?}");
    assert_eq!(tag_count(&dir, "profile"), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// DONE never enters any child's environment: a command run after the
/// startup chain cannot observe it (pre-fix it expanded to `1`).
#[test]
fn startup_done_marker_not_visible_to_commands() {
    let dir = counting_home("done-invisible");
    let run = spawn(
        &["-lc", "echo DONE=${__RUBASH_STARTUP_FILES_DONE-UNSET}"],
        &dir,
        &[],
        b"",
    );
    assert!(!run.timed_out);
    assert!(run.stdout.contains("DONE=UNSET"), "{run:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// An inherited DONE copy is stripped at entry, not honored: an own
/// `-lc` still sources its profile.
#[cfg(not(windows))]
#[test]
fn inherited_done_does_not_suppress_own_login() {
    let dir = counting_home("inh-done");
    let run = spawn(
        &["-lc", "true"],
        &dir,
        &[("__RUBASH_STARTUP_FILES_DONE", "1")],
        b"",
    );
    assert!(!run.timed_out);
    assert_eq!(tag_count(&dir, "profile"), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// #2133/F6: suppressors are armed by own argv only
// ---------------------------------------------------------------------------

/// An inherited `__RUBASH_NO_PROFILE` must not defeat an explicit
/// `-l` on this process's own argv (pre-fix the profile count was 0).
#[cfg(not(windows))]
#[test]
fn inherited_no_profile_does_not_defeat_own_login() {
    let dir = counting_home("inh-noprofile");
    let run = spawn(&["-lc", "true"], &dir, &[("__RUBASH_NO_PROFILE", "1")], b"");
    assert!(!run.timed_out);
    assert_eq!(tag_count(&dir, "profile"), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// An inherited `__RUBASH_NO_RC` must not defeat an own `-i`.
#[test]
fn inherited_no_rc_does_not_defeat_own_interactive() {
    let dir = counting_home("inh-norc");
    let run = spawn(&["-ic", "true"], &dir, &[("__RUBASH_NO_RC", "1")], b"");
    assert!(!run.timed_out);
    assert_eq!(tag_count(&dir, "rc"), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The other direction: this process's own --noprofile/--norc still
/// suppress exactly their own branches.
#[test]
fn own_argv_suppressors_still_work() {
    let dir = counting_home("own-noprofile");
    let run = spawn(&["--noprofile", "-lc", "true"], &dir, &[], b"");
    assert!(!run.timed_out);
    assert_eq!(tag_count(&dir, "profile"), 0);
    let _ = std::fs::remove_dir_all(&dir);

    let dir = counting_home("own-norc");
    let run = spawn(&["--norc", "-ic", "true"], &dir, &[], b"");
    assert!(!run.timed_out);
    assert_eq!(tag_count(&dir, "rc"), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// $BASH_ENV face: exactly once, and respawns source it zero times
// ---------------------------------------------------------------------------

/// Non-interactive shells source $BASH_ENV exactly once even under
/// inherited markers; the command substitution inside the env file is
/// a respawned child and must not source it again (pre-fix the
/// markers hijacked the branch to the profile chain and the env file
/// never ran at all on this shape).
#[test]
fn bash_env_sourced_exactly_once_under_markers() {
    let dir = counting_home("bashenv");
    let count = dir.join("count").to_string_lossy().replace('\\', "/");
    // The comsub is the respawn probe: if the respawned child re-ran
    // the startup chain, the count would climb past 1.
    write_file(
        &dir,
        "bashenv",
        &format!("echo bashenv >> \"{count}\"\n: \"$(true)\"\n"),
    );
    let bash_env = dir.join("bashenv").to_string_lossy().replace('\\', "/");
    let run = spawn(
        &["-c", "true"],
        &dir,
        &[
            ("BASH_ENV", bash_env.as_str()),
            BOTH_MARKERS[0],
            BOTH_MARKERS[1],
        ],
        b"",
    );
    assert!(!run.timed_out, "must terminate: {run:?}");
    assert_eq!(tag_count(&dir, "bashenv"), 1);
    assert_eq!(tag_count(&dir, "profile"), 0);
    assert_eq!(tag_count(&dir, "rc"), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Logout face: the carrier drop reaches the exit door
// ---------------------------------------------------------------------------

/// An own login shell's `logout` builtin sources ~/.bash_logout once.
#[test]
fn own_login_logout_sources_logout_file() {
    let dir = counting_home("logout-own");
    let run = spawn(&["-lc", "logout"], &dir, &[], b"");
    assert!(!run.timed_out);
    assert_eq!(run.code, Some(0), "{run:?}");
    assert_eq!(tag_count(&dir, "logout"), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A `-c` child that merely inherited the login carrier is NOT a
/// login shell at the exit door either: `logout` refuses (GNU
/// exit.def:79-90) and ~/.bash_logout never runs (pre-fix the
/// inherited carrier made it source the file and exit 0).
#[test]
fn inherited_login_logout_is_refused() {
    let dir = counting_home("logout-inherited");
    let run = spawn(&["-c", "logout"], &dir, BOTH_MARKERS, b"");
    assert!(!run.timed_out);
    assert_eq!(run.code, Some(1), "{run:?}");
    assert_eq!(tag_count(&dir, "logout"), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// The fzf shape end to end
// ---------------------------------------------------------------------------

/// An interactive shell whose rc carries fzf's completion procsub
/// line completes startup exactly once and answers commands (pre-fix
/// composition: every generation re-sourced and respawned).
#[test]
fn fzf_shape_interactive_startup_completes_once() {
    let dir = home("fzf");
    let count = dir.join("count").to_string_lossy().replace('\\', "/");
    write_file(
        &dir,
        ".bashrc",
        &format!(
            "echo rc >> \"{count}\"\n\
             __fzf_orig_completion() {{ cat > /dev/null; }}\n\
             __fzf_orig_completion < <(complete -p $d_cmds $a_cmds $v_cmds unalias kill ssh 2> /dev/null)\n"
        ),
    );
    write_file(&dir, "count", "");
    let run = spawn(&["-i"], &dir, &[], b"echo ALIVE\n");
    assert!(!run.timed_out, "fzf-shaped startup must finish: {run:?}");
    assert_eq!(tag_count(&dir, "rc"), 1);
    assert!(run.stdout.contains("ALIVE"), "{run:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
