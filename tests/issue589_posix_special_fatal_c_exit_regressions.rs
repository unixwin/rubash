//! rubash#589: a POSIX-mode special-builtin/assignment violation terminates
//! the shell with the right code for every entry shape.
//!
//! GNU anchors: subst.c:13177-13182 (expand_word_list_internal, is_nullcmd
//! arm) and subst.c:13186-13195 (tempenv arm,
//! bash_variable_assignment_error) jump FORCE_EOF through
//! posix_variable_assignment_error (subst.c:11181-11196); execute_cmd.c:3085-3089
//! (execute_for_command) and execute_cmd.c:3587-3592
//! (execute_select_command) jump FORCE_EOF on a readonly loop/select bind.
//! The status is context-owned: shell.c:1471-1472 run_one_command maps
//! `case FORCE_EOF: return last_command_exit_value = 127` for `-c`, while
//! eval.c:104-109 (reader_loop) ends script input and exits with
//! last_command_exit_value (1), and execute_cmd.c:1811 contains the jump at
//! 1 inside a subshell child. rubash used to exit 1 on every shape.
//!
//! WSL GNU 5.3.0 oracle (2026-10-13, `wsl -e bash --posix -c ...`):
//! nullcmd `ro=2` 127 / `ro=2 :` 127 / `ro=2 true` 0 + after / for-bind 127 /
//! select-bind 127 / `(ro=2)` parent $? = 1, overall 0 / `x=${ro:=v}` 0 +
//! after / `read ro` 0 + after / stdin-script 1 / script file 1.

use std::io::Write;
use std::process::{Command, Stdio};

fn run(args: &[&str], stdin: &[u8]) -> (String, String, Option<i32>) {
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

/// `bash --posix -c 'readonly ro=1\nro=2\necho after'` exits 127 — the
/// shell.c:1471 run_one_command FORCE_EOF arm — and never runs `after`.
#[test]
fn posix_c_nullcmd_readonly_fatal_exits_127() {
    let (stdout, stderr, rc) = run(&["--posix", "-c", "readonly ro=1\nro=2\necho after\n"], b"");
    assert_eq!(rc, Some(127), "stderr: {stderr:?}");
    assert_eq!(stdout, "", "the tail must not run");
    assert!(stderr.contains("readonly variable"), "stderr: {stderr:?}");
}

/// Same fatal on the SAME LINE (`ro=2; echo post`) keeps the 127 mapping.
#[test]
fn posix_c_same_line_fatal_exits_127() {
    let (stdout, stderr, rc) = run(
        &[
            "--posix",
            "-c",
            "readonly ro=1\necho pre; ro=2; echo post\n",
        ],
        b"",
    );
    assert_eq!(rc, Some(127), "stderr: {stderr:?}");
    assert_eq!(stdout, "pre\n", "only the pre-statement runs");
    assert!(stderr.contains("readonly variable"), "stderr: {stderr:?}");
}

/// A failed readonly tempenv assignment before a SPECIAL builtin (`:`)
/// exits 127 under -c (subst.c:13186-13195 -> FORCE_EOF).
#[test]
fn posix_c_tempenv_special_builtin_fatal_exits_127() {
    let (stdout, stderr, rc) = run(
        &["--posix", "-c", "readonly ro=1\nro=2 :\necho after\n"],
        b"",
    );
    assert_eq!(rc, Some(127), "stderr: {stderr:?}");
    assert_eq!(stdout, "", "the tail must not run");
    assert!(stderr.contains("readonly variable"), "stderr: {stderr:?}");
}

/// Before an ORDINARY builtin the failure is not fatal: the command is
/// skipped and the rest runs (GNU: `ro=2 true` -> after, rc 0).
#[test]
fn posix_c_tempenv_ordinary_builtin_is_not_fatal() {
    let (stdout, stderr, rc) = run(
        &["--posix", "-c", "readonly ro=1\nro=2 true\necho after\n"],
        b"",
    );
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
    assert_eq!(stdout, "after\n");
    assert!(stderr.contains("readonly variable"), "stderr: {stderr:?}");
}

/// A readonly `for` variable bind is fatal under posix -c
/// (execute_cmd.c:3085-3089 FORCE_EOF): no iterations, no tail, exit 127.
#[test]
fn posix_c_for_readonly_bind_fatal_exits_127() {
    let (stdout, stderr, rc) = run(
        &[
            "--posix",
            "-c",
            "readonly ro=1\nfor ro in 1 2; do echo loop-$ro; done\necho after\n",
        ],
        b"",
    );
    assert_eq!(rc, Some(127), "stderr: {stderr:?}");
    assert_eq!(stdout, "", "neither the body nor the tail runs");
    assert!(stderr.contains("readonly variable"), "stderr: {stderr:?}");
}

/// A readonly `select` variable bind is fatal under posix -c
/// (execute_cmd.c:3587-3592 FORCE_EOF): exit 127, no tail.
#[test]
fn posix_c_select_readonly_bind_fatal_exits_127() {
    let (stdout, stderr, rc) = run(
        &[
            "--posix",
            "-c",
            "readonly ro=1\nselect ro in a b; do break; done\necho after\n",
        ],
        b"1\n",
    );
    assert_eq!(rc, Some(127), "stderr: {stderr:?}");
    assert!(
        !stdout.contains("after"),
        "the tail must not run; stdout: {stdout:?}"
    );
    assert!(stderr.contains("readonly variable"), "stderr: {stderr:?}");
}

/// The jump is contained inside a subshell child (execute_cmd.c:1811): the
/// child exits 1, the parent keeps running and the shell exits 0.
#[test]
fn posix_c_subshell_contains_the_fatal_jump() {
    let (stdout, stderr, rc) = run(
        &[
            "--posix",
            "-c",
            "readonly ro=1\n(ro=2)\necho \"parent-after\"\n",
        ],
        b"",
    );
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
    assert_eq!(stdout, "parent-after\n");
    assert!(stderr.contains("readonly variable"), "stderr: {stderr:?}");
}

/// Script-file entry keeps the reader-loop mapping: exit 1, tail not run
/// (eval.c:104-109 reader_loop FORCE_EOF -> last_command_exit_value).
#[test]
fn posix_script_file_fatal_exits_1() {
    let script = std::env::temp_dir().join("issue589_posix_script.sh");
    std::fs::write(&script, "readonly ro=1\nro=2\necho after\n").expect("write script");
    let (stdout, stderr, rc) = run(&["--posix", &script.to_string_lossy()], b"");
    let _ = std::fs::remove_file(&script);
    assert_eq!(rc, Some(1), "stderr: {stderr:?}");
    assert_eq!(stdout, "", "the tail must not run");
    assert!(stderr.contains("readonly variable"), "stderr: {stderr:?}");
}

/// Stdin-script entry (-s) agrees with the script-file shape: exit 1.
#[test]
fn posix_stdin_script_fatal_exits_1() {
    let (stdout, stderr, rc) = run(&["--posix", "-s"], b"readonly ro=1\nro=2\necho after\n");
    assert_eq!(rc, Some(1), "stderr: {stderr:?}");
    assert_eq!(stdout, "", "the tail must not run");
    assert!(stderr.contains("readonly variable"), "stderr: {stderr:?}");
}

/// Without --posix nothing is fatal: the diagnostic prints, the tail runs,
/// the shell exits with the last command's status (GNU probe: rc 0).
#[test]
fn default_mode_nullcmd_failure_is_not_fatal() {
    let (stdout, stderr, rc) = run(&["-c", "readonly ro=1\nro=2\necho after\n"], b"");
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
    assert_eq!(stdout, "after\n");
    assert!(stderr.contains("readonly variable"), "stderr: {stderr:?}");
}
