//! Issue 2393: a script-file invocation must read $BASH_ENV.
//!
//! GNU shell.c:722 run_startup_files runs for every shell
//! invocation — `-c`, `-s`, and script file alike — and its
//! non-interactive branch (shell.c:1214-1220) sources $BASH_ENV.
//! When a running rubash invokes its own binary with a script
//! operand (`"$0" scr.sh`, the ${THIS_SH} shape), the command runs
//! as an in-process fresh-shell child; that child's initialization
//! must run the same env-file step or the script loses the file's
//! variables, its functions, and its `exit` semantics.
//!
//! Oracle: GNU bash 5.3.20, script-file probes (see the issue's
//! Regression seed: script-su12=twelve; a BASH_ENV-defined function
//! is callable; `exit 3` in BASH_ENV aborts with rc 3 and the script
//! body never runs).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn rubash_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rubash"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2393-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn write(dir: &Path, name: &str, contents: &str) {
    fs::write(dir.join(name), contents).expect("write scratch file");
}

/// Run `driver.sh` under rubash with $1 = scratch dir, $2 = the
/// rubash binary path (the stand-in for the seed's `"$0"`).
fn run_driver(dir: &Path) -> (String, Option<i32>) {
    let output = Command::new(rubash_bin())
        .arg("driver.sh")
        .arg(dir)
        .arg(rubash_bin())
        .current_dir(dir)
        .env("HOME", dir)
        .output()
        .expect("run driver under rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue2393_same_shell_script_child_reads_bash_env() {
    let dir = scratch("vars");
    write(&dir, "env12.sh", "SU12=twelve\n");
    write(&dir, "scr12.sh", "echo script-su12=${SU12-unset}\n");
    write(&dir, "driver.sh", "BASH_ENV=$1/env12.sh \"$2\" scr12.sh\n");
    let (stdout, code) = run_driver(&dir);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "script-su12=twelve\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2393_bash_env_function_is_callable_from_script() {
    let dir = scratch("fn");
    write(&dir, "envf.sh", "myfn() { echo fn-called; }\n");
    write(&dir, "scrf.sh", "myfn\n");
    write(&dir, "driver.sh", "BASH_ENV=$1/envf.sh \"$2\" scrf.sh\n");
    let (stdout, code) = run_driver(&dir);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "fn-called\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2393_bash_env_exit_aborts_before_script_body() {
    let dir = scratch("exit");
    write(&dir, "envexit.sh", "exit 3\n");
    write(&dir, "scrbody.sh", "echo body-ran\n");
    write(
        &dir,
        "driver.sh",
        "BASH_ENV=$1/envexit.sh \"$2\" scrbody.sh\n",
    );
    let (stdout, code) = run_driver(&dir);
    assert_eq!(code, Some(3));
    assert_eq!(stdout, "");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2393_direct_script_invocation_reads_bash_env_control() {
    let dir = scratch("direct");
    write(&dir, "env12.sh", "SU12=twelve\n");
    write(&dir, "scr12.sh", "echo script-su12=${SU12-unset}\n");
    let output = Command::new(rubash_bin())
        .arg("scr12.sh")
        .current_dir(&dir)
        .env("BASH_ENV", dir.join("env12.sh"))
        .env("HOME", &dir)
        .output()
        .expect("run rubash scr12.sh");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).into_owned(),
        "script-su12=twelve\n"
    );
    let _ = fs::remove_dir_all(&dir);
}
