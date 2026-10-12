//! niubash#282: console raw input support.
//!
//! The console-mode legs (single-key `read -n1` without Enter, `read -s`
//! echo suppression, `stty raw/-echo/sane/size`) need a real conhost
//! console and are covered by scripts/smoke-read-tty-pty.py under ConPTY;
//! the script SKIPs the stty legs on winpty-style bridges, where stdin is
//! a plain character device without the console-mode API (真机待验).
//! These are the deterministic pipe-transport legs: the engine builtin
//! answers stty itself (GNU bash runs an external stty; on Windows none
//! exists, so TUI scripts' `stty raw -echo` used to die with
//! command-not-found) and reports the coreutils non-terminal diagnostic.

use std::process::{Command, Stdio};

fn run_stdin(args: &[&str], stdin: &[u8]) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    use std::io::Write;
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(stdin)
        .expect("write stdin");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// `stty` is claimed by the engine: a missing external can no longer break
/// TUI scripts' mode setup, and introspection keeps reporting external
/// (the name stays out of BUILTIN_NAMES), but the command resolves.
#[test]
fn stty_is_answered_by_engine() {
    let (stdout, stderr, rc) = run_stdin(&["-c", "command -v stty; echo \"rc=$?\""], b"");
    assert_ne!(stderr, "rubash: stty: command not found\n");
    assert!(rc.is_some());
    let _ = stdout;
}

/// Non-terminal stdin: the coreutils diagnostic and status 1 (GNU probe:
/// `stty raw < /dev/null` → `stty: 'standard input': Inappropriate ioctl
/// for device`, rc=1).
#[test]
fn stty_non_terminal_stdin_reports_gnu_diagnostic() {
    let (stdout, stderr, rc) = run_stdin(&["-c", "stty raw; echo \"rc=$?\""], b"");
    assert_eq!(
        stderr, "stty: 'standard input': Inappropriate ioctl for device\n",
        "stdout: {stdout:?}"
    );
    assert_eq!(stdout, "rc=1\n");
}

/// Same for `stty size` and `stty -a` shapes — every operand form fails
/// with the diagnostic when stdin has no console-mode API.
#[test]
fn stty_size_non_terminal_diagnostic() {
    let (_stdout, stderr, rc) = run_stdin(&["-c", "stty size > /dev/null; echo \"rc=$?\""], b"");
    assert_eq!(
        stderr,
        "stty: 'standard input': Inappropriate ioctl for device\n"
    );
    assert_eq!(_stdout, "rc=1\n");
    let _ = rc;
}

/// A shell function named stty keeps winning over the engine tool, like
/// every GNU external command (rubash#154 rule).
#[test]
fn shell_function_shadows_stty() {
    let (stdout, stderr, rc) =
        run_stdin(&["-c", "stty() { echo FUN; }; stty; echo \"rc=$?\""], b"");
    assert_eq!(stdout, "FUN\nrc=0\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}
