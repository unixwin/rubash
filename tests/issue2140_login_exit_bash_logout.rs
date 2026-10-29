//! Issue rubash#2140 regressions: a GNU login shell's `exit` builtin
//! sources ~/.bash_logout, not only `logout`.
//!
//! GNU ownership: builtins/exit.def `exit_or_logout` calls
//! `bash_logout()` (exit.def:147) after the exit arguments validate —
//! an argument error returns first and never reaches it — and
//! `bash_logout` itself (exit.def:156-166) sources ~/.bash_logout
//! only when `login_shell && sourced_logout++ == 0 &&
//! subshell_environment == 0`. Every expectation below was probed
//! against GNU bash 5.3.20 with a counting ~/.bash_logout:
//!
//!   -lc 'exit'              -> 1 run, rc 0
//!   -lc 'logout'            -> 1 run (the pre-existing correct face)
//!   -c  'exit'              -> 0 (not a login shell)
//!   -lc 'exit 3'            -> 1 run, rc 3
//!   -lc 'true'              -> 0 (implicit end is not the builtin)
//!   -lc '(exit)'            -> 0 (subshell_environment != 0)
//!   -lc '(exit); exit'      -> 1 (the subshell does not spend the latch)
//!   -lc 'f(){ exit; }; f'   -> 1
//!   -lc 'exit abc'          -> 0 (argument error returns before bash_logout)
//!   -lc 'exit | cat; exit'  -> 1 (pipeline segment is a fork copy)
//!   -lc 'exec true'         -> 0 (exec is not the exit builtin)
//!
//! Each probe counts ~/.bash_logout executions in a counter file and
//! is watchdog-bounded so a regression fails by deadline, never by
//! hanging the suite.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const WATCHDOG: Duration = Duration::from_secs(20);

#[derive(Debug)]
struct Run {
    code: Option<i32>,
    timed_out: bool,
}

fn home(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i2140-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch home");
    dir
}

fn bin() -> String {
    env!("CARGO_BIN_EXE_rubash").replace('\\', "/")
}

/// A home whose ~/.bash_logout appends one line to `count` per run.
fn counting_home(tag: &str, logout_extra: &str) -> PathBuf {
    let dir = home(tag);
    let count = dir.join("count").to_string_lossy().replace('\\', "/");
    std::fs::write(
        dir.join(".bash_logout"),
        format!("echo logout >> \"{count}\"\n{logout_extra}"),
    )
    .expect("write .bash_logout");
    dir
}

fn logout_count(dir: &Path) -> usize {
    std::fs::read_to_string(dir.join("count"))
        .unwrap_or_default()
        .lines()
        .filter(|line| *line == "logout")
        .count()
}

/// Spawn the binary with a scrubbed environment (the whole
/// startup-carrier family removed, per the issue2129 convention) and
/// a hard deadline after which the child is killed.
fn spawn(args: &[&str], home_dir: &Path) -> Run {
    let scratch = home("io");
    let out_path = scratch.join("stdout");
    let stdout_file = std::fs::File::create(&out_path).expect("stdout file");
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
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::null());
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
    let _ = std::fs::remove_dir_all(&scratch);
    Run {
        code,
        timed_out: code.is_none(),
    }
}

fn assert_logout_runs(tag: &str, script_args: &[&str], expected: usize) -> Run {
    let dir = counting_home(tag, "");
    let run = spawn(script_args, &dir);
    assert!(!run.timed_out, "{tag}: shell did not terminate");
    assert_eq!(
        logout_count(&dir),
        expected,
        "{tag}: ~/.bash_logout run count for {script_args:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
    run
}

#[test]
fn issue2140_login_exit_sources_bash_logout() {
    let run = assert_logout_runs("login-exit", &["-lc", "exit"], 1);
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue2140_login_logout_still_sources_bash_logout() {
    assert_logout_runs("login-logout", &["-lc", "logout"], 1);
}

#[test]
fn issue2140_non_login_exit_skips_bash_logout() {
    assert_logout_runs("plain-exit", &["-c", "exit"], 0);
}

#[test]
fn issue2140_login_exit_with_status_sources_bash_logout() {
    let run = assert_logout_runs("login-exit-3", &["-lc", "exit 3"], 1);
    assert_eq!(run.code, Some(3));
}

#[test]
fn issue2140_login_implicit_end_skips_bash_logout() {
    assert_logout_runs("login-implicit", &["-lc", "true"], 0);
}

#[test]
fn issue2140_login_subshell_exit_skips_bash_logout() {
    assert_logout_runs("login-subshell", &["-lc", "(exit)"], 0);
}

#[test]
fn issue2140_login_subshell_exit_does_not_spend_the_latch() {
    assert_logout_runs("login-subshell-then-exit", &["-lc", "(exit); exit"], 1);
}

#[test]
fn issue2140_login_exit_in_function_sources_bash_logout() {
    assert_logout_runs("login-function-exit", &["-lc", "f(){ exit; }; f"], 1);
}

#[test]
fn issue2140_login_exit_argument_error_skips_bash_logout() {
    assert_logout_runs("login-exit-abc", &["-lc", "exit abc"], 0);
}

#[test]
fn issue2140_login_pipeline_exit_then_exit_sources_once() {
    assert_logout_runs("login-pipeline", &["-lc", "exit | cat; exit"], 1);
}

#[test]
fn issue2140_login_exec_skips_bash_logout() {
    assert_logout_runs("login-exec", &["-lc", "exec true"], 0);
}

#[test]
fn issue2140_bash_logout_calling_exit_does_not_recurse() {
    let dir = counting_home("logout-reexit", "exit\n");
    let run = spawn(&["-lc", "exit"], &dir);
    assert!(!run.timed_out, "bash_logout calling exit must terminate");
    assert_eq!(logout_count(&dir), 1);
    let _ = std::fs::remove_dir_all(&dir);
}
