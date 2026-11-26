//! rubash#591: `-s` (stdin-script) shows `s` in `$-`.
//!
//! GNU anchors: flags.c:294-306 which_set_flags appends `s` while
//! `read_from_stdin` (shell.c:301) is set; shell.c:928 records the explicit
//! `-s` at option-parse time, shell.c:779-786 records the non-interactive
//! no-operand stdin reader. WSL GNU 5.3.0 oracle (2026-10-13):
//! `printf 'echo [$-]' | bash -s` prints `[hBs]`; `bash -c 'echo [$-]'`
//! prints `[hBc]` (no s). rubash's engine drivers armed the marker only
//! inside run_stdin_script / run_interactive_stdin, so a library host
//! driving its own stdin reader (niu) lost the letter; the shared
//! invocation surface (ShellInvocation::apply_to_executor) now arms it for
//! every `-s` invocation.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_stdin(args: &[&str], stdin: &[u8]) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(args)
        .env_remove("BASH_ARGV0")
        .env_remove("NIU_INVOKED_AS")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
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

/// `printf 'echo [$-]' | niu -s` -> `[hBs]` (GNU 5.3.0 oracle byte-for-byte).
#[test]
fn explicit_s_flag_shows_s_in_dollar_minus() {
    let (stdout, stderr, rc) = run_stdin(&["-s"], b"echo \"[$-]\"");
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
    assert_eq!(stdout, "[hBs]\n", "stderr: {stderr:?}");
}

/// The no-operand piped entry carries `s` too: shell.c:779-786 sets
/// read_from_stdin for the non-interactive stdin reader (GNU: `printf
/// 'echo [$-]' | bash` -> `[hBs]`).
#[test]
fn no_operand_piped_stdin_shows_s_in_dollar_minus() {
    let (stdout, stderr, rc) = run_stdin(&[], b"echo \"[$-]\"\n");
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
    assert_eq!(stdout, "[hBs]\n", "stderr: {stderr:?}");
}

/// `-c` keeps GNU's shape without `s`: `[hBc]` (want_pending_command only).
#[test]
fn command_string_has_no_s_in_dollar_minus() {
    let (stdout, stderr, rc) = run_stdin(&["-c", "echo \"[$-]\""], b"");
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
    assert_eq!(stdout, "[hBc]\n", "stderr: {stderr:?}");
}

/// Script-file entry carries neither `c` nor `s` (GNU `bash script` shows
/// the plain option letters; shell_execscript never sets read_from_stdin).
#[test]
fn script_file_has_no_s_or_c_in_dollar_minus() {
    let script = std::env::temp_dir().join("issue591_script.sh");
    std::fs::write(&script, "echo \"[$-]\"\n").expect("write script");
    let (stdout, stderr, rc) = run_stdin(&[&script.to_string_lossy()], b"");
    let _ = std::fs::remove_file(&script);
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
    assert_eq!(stdout, "[hB]\n", "stderr: {stderr:?}");
}

/// The `s` letter never reaches a child: which_set_flags' read_from_stdin
/// is a process-local C global, and the __RUBASH_ env filter keeps the
/// marker internal. A child `rubash -c 'echo [$-]'` spawned from a `-s`
/// session reports the plain letters (GNU: `printf '"bash" -c "echo [\\$-]"'
/// | bash -s` -> `[hBc]`).
#[test]
fn s_flag_stays_process_local() {
    let (stdout, stderr, rc) = run_stdin(
        &["-s"],
        format!(
            "\"{}\" -c 'echo \"[$-]\"'\n",
            env!("CARGO_BIN_EXE_rubash").replace('\\', "/")
        )
        .as_bytes(),
    );
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
    assert_eq!(stdout, "[hBc]\n", "stderr: {stderr:?}");
}

/// The library contract the niu host depends on: parsing `-s` and applying
/// the invocation arms rubash's read-from-stdin marker on the executor, so
/// `$-` shows `s` whatever driver serves the input. `-c` arms no marker.
#[test]
fn apply_to_executor_arms_read_stdin_marker() {
    // Mirror the login-marker tests: reset ambient markers so a leaked
    // process env cannot fake a pass.
    std::env::remove_var(rubash::script_driver::READ_STDIN_MARKER);
    let mut executor = rubash::executor::Executor::new_process_exit();
    let args: Vec<String> = ["-s"].iter().map(|s| s.to_string()).collect();
    let invocation = rubash::invocation::ShellInvocation::parse(&args).expect("parse -s");
    invocation
        .apply_to_executor(&mut executor)
        .expect("apply -s invocation");
    assert_eq!(
        executor.get_env(rubash::script_driver::READ_STDIN_MARKER),
        Some("1"),
        "explicit -s must arm the read-from-stdin marker"
    );

    std::env::remove_var(rubash::script_driver::READ_STDIN_MARKER);
    let mut executor = rubash::executor::Executor::new_process_exit();
    let args: Vec<String> = ["-c", "true"].iter().map(|s| s.to_string()).collect();
    let invocation = rubash::invocation::ShellInvocation::parse(&args).expect("parse -c");
    invocation
        .apply_to_executor(&mut executor)
        .expect("apply -c invocation");
    assert_eq!(
        executor.get_env(rubash::script_driver::READ_STDIN_MARKER),
        None,
        "-c must not arm the read-from-stdin marker"
    );
}
