//! Issue rubash#2135 regressions: the function-stdin staging pair is
//! execution state and is never imported from the ambient environment.
//!
//! The defect: `__RUBASH_FUNCTION_STDIN` (with its offset companion)
//! stages a substitute stdin for function calls, and every read site
//! consults the live environment table. An ambient copy in a fresh
//! process's environment was imported untouched, so a parent exporting
//! `__RUBASH_FUNCTION_STDIN=<file>` hijacked the child's stdin surface:
//! the first function-call `read` consumed the named file instead of the
//! real stdin, and so did external commands in the staged scope.
//!
//! The fix joins the existing import strip-list (the
//! `__RUBASH_CURRENT_FUNCTION`/`__RUBASH_IN_SOURCE` precedent): both
//! keys are removed from the imported table and from the process-env
//! mirror at `Executor::new`. Every legitimate write is in-process and
//! post-import (the function-call stdin scope, the `source` builtin's
//! explicit-path staging), so genuine function-stdin faces — the
//! rubash#2111 surface — are unaffected.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const WATCHDOG: Duration = Duration::from_secs(20);

fn bin() -> String {
    env!("CARGO_BIN_EXE_rubash").replace('\\', "/")
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i2135-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

struct Run {
    stdout: String,
    code: Option<i32>,
    home: PathBuf,
}

fn spawn(args: &[&str], extra_env: &[(&str, &str)], stdin: &[u8]) -> Run {
    let dir = scratch("io");
    let in_path = dir.join("stdin");
    std::fs::write(&in_path, stdin).expect("write stdin");
    let mut command = Command::new(bin());
    command
        .args(args)
        .env("HOME", dir.to_string_lossy().replace('\\', "/"))
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .env_remove("__RUBASH_FUNCTION_STDIN")
        .env_remove("__RUBASH_FUNCTION_STDIN_OFFSET")
        .stdin(Stdio::from(
            std::fs::File::open(&in_path).expect("stdin file"),
        ))
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let mut child = command.spawn().expect("spawn rubash");
    let deadline = Instant::now() + WATCHDOG;
    let code = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break Some(status.code().unwrap_or(-1)),
            None => {
                assert!(Instant::now() < deadline, "watchdog: child hung");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    };
    let mut stdout = String::new();
    if let Some(mut out) = child.stdout.take() {
        use std::io::Read;
        let _ = out.read_to_string(&mut stdout);
    }
    Run {
        stdout: stdout.trim().to_string(),
        code,
        home: dir,
    }
}

fn forged_file() -> (PathBuf, String) {
    let dir = scratch("forge");
    let path = dir.join("forged-stdin");
    std::fs::write(&path, "FORGED_LINE\n").expect("write forged file");
    let value = path.to_string_lossy().replace('\\', "/");
    (dir, value)
}

#[test]
fn forged_staging_does_not_hijack_function_read() {
    let (_dir, forged) = forged_file();
    let run = spawn(
        &["-c", "f() { read line; echo \"got:$line\"; }; f"],
        &[("__RUBASH_FUNCTION_STDIN", &forged)],
        b"REAL_LINE\n",
    );
    assert_eq!(run.code, Some(0));
    assert_eq!(run.stdout, "got:REAL_LINE");
    let _ = run.home;
}

#[test]
fn forged_staging_does_not_hijack_external_command_in_function() {
    let (_dir, forged) = forged_file();
    let run = spawn(
        &["-c", "f() { cat; }; f"],
        &[("__RUBASH_FUNCTION_STDIN", &forged)],
        b"REAL_LINE\n",
    );
    assert_eq!(run.stdout, "REAL_LINE");
}

#[test]
fn control_function_read_uses_real_stdin() {
    let run = spawn(
        &["-c", "f() { read line; echo \"got:$line\"; }; f"],
        &[],
        b"REAL_LINE\n",
    );
    assert_eq!(run.stdout, "got:REAL_LINE");
}

#[test]
fn genuine_pipe_into_function_still_stages() {
    let run = spawn(
        &[
            "-c",
            "f() { read x; echo \"got:$x\"; }; printf 'PIPE_LINE\\n' | f",
        ],
        &[],
        b"",
    );
    assert_eq!(run.stdout, "got:PIPE_LINE");
}

#[test]
fn genuine_while_read_loop_still_stages() {
    let run = spawn(
        &[
            "-c",
            "printf 'a\\nb\\n' | while read l; do echo \"L=$l\"; done",
        ],
        &[],
        b"",
    );
    assert_eq!(run.stdout, "L=a\nL=b");
}

#[test]
fn genuine_herestring_into_function_still_stages() {
    let run = spawn(
        &["-c", "f() { read x; echo \"got:$x\"; }; f <<< \"HS_LINE\""],
        &[],
        b"",
    );
    assert_eq!(run.stdout, "got:HS_LINE");
}
