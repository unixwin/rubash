//! rubash#3964 (face 2): `cat /dev/stdin` (and the `/dev/fd/0`
//! spelling) as an OPERAND failed with
//! `cat: /dev/stdin: No such file or directory` whenever the
//! current context's stdin rides the FUNCTION_STDIN channel —
//! a function body, a subshell or brace-group pipeline stage, a
//! `command cat` stage — while a plain pipeline stage and
//! file-bound positions worked. The in-process cat resolves the
//! operand through dev_fd_operand_bytes, whose fd-0 serve only
//! knew the fd table: in channel contexts the table carries no
//! fd-0 entry, so the serve returned None and the caller reported
//! ENOENT. GNU hands the child a dup of the descriptor in effect,
//! so the operand reads the context's stream, sharing its offset
//! with `read` and with cat's own `-` operand.
//!
//! Oracle: GNU bash 5.3.20, script files, byte-compared
//! (stdout/stderr/status). The report is from Windows; the
//! mechanism is the platform-independent operand serve (the same
//! in-process cat runs there), verified here on Linux.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3964b-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    std::fs::write(dir.join("data.txt"), "filedata\n").expect("write data.txt");
    dir
}

fn rubash_c(script: &str, dir: &Path) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn assert_run(script: &str, expected_stdout: &str) {
    let dir = tempdir("case");
    let run = rubash_c(script, &dir);
    assert_eq!(run.stdout, expected_stdout, "stdout for {script:?}");
    assert_eq!(run.stderr, "", "stderr for {script:?}");
    assert_eq!(run.code, Some(0), "status for {script:?}");
}

/// The function position from the report.
#[test]
fn cat_dev_stdin_operand_in_function_stage() {
    assert_run("g(){ cat /dev/stdin; }; echo a | g", "a\n");
}

/// The subshell position from the report.
#[test]
fn cat_dev_stdin_operand_in_subshell_stage() {
    assert_run("echo b | (cat /dev/stdin)", "b\n");
}

/// The `command cat` position from the report.
#[test]
fn cat_dev_stdin_operand_under_command_prefix() {
    assert_run("echo c | command cat /dev/stdin", "c\n");
}

/// A brace-group pipeline stage carries its input on the same
/// channel.
#[test]
fn cat_dev_stdin_operand_in_group_stage() {
    assert_run("echo e | { cat /dev/stdin; }", "e\n");
}

/// The `/dev/fd/0` spelling names the same fd 0.
#[test]
fn cat_dev_fd0_operand_in_function_stage() {
    assert_run("g(){ cat /dev/fd/0; }; echo f | g", "f\n");
}

/// Shared cursor: `read` consumes the first line; the operand
/// sees only the remainder.
#[test]
fn cat_dev_stdin_operand_after_read_shares_cursor() {
    assert_run(
        "f(){ read a; cat /dev/stdin; }; printf 'l1\\nl2\\n' | f",
        "l2\n",
    );
}

/// The operand drains the context stream: a later `read` in the
/// same function sees EOF.
#[test]
fn cat_dev_stdin_operand_drains_for_later_read() {
    assert_run(
        "f(){ cat /dev/stdin; read a; echo \"read=[$a]\"; }; printf 'l1\\nl2\\n' | f",
        "l1\nl2\nread=[]\n",
    );
}

/// Two fd-0 operands in one invocation: the first drains the
/// stream, the second reads EOF — the payload appears once.
#[test]
fn cat_dev_stdin_operand_twice_reads_once() {
    assert_run("g(){ cat /dev/stdin /dev/stdin; }; echo a | g", "a\n");
}

/// The operand before `-`: the operand drains, the `-` operand
/// then reads EOF (no diagnostic, status 0).
#[test]
fn cat_dev_stdin_operand_before_dash() {
    assert_run("g(){ cat /dev/stdin -; }; echo a | g", "a\n");
}

/// Control: a plain pipeline stage already worked (its operand
/// resolution rides the stage-input cursor).
#[test]
fn cat_dev_stdin_operand_plain_stage_control() {
    assert_run("echo d | cat /dev/stdin", "d\n");
}

/// Control: a top-level here-string binds fd 0 in the fd table.
#[test]
fn cat_dev_stdin_operand_top_level_herestring_control() {
    assert_run("cat /dev/stdin <<<top", "top\n");
}

/// Control: an enclosing redirect that rebinds fd 0 to a real
/// file shadows the channel.
#[test]
fn cat_dev_stdin_operand_group_redirect_file_shadows_channel() {
    assert_run(
        "f(){ { cat /dev/stdin; } < data.txt; }; f <<<hi",
        "filedata\n",
    );
}

/// Control: the command's own `< file` redirect wins over the
/// function's stdin.
#[test]
fn cat_dev_stdin_operand_own_redirect_control() {
    assert_run("f(){ cat /dev/stdin < data.txt; }; f <<<hi", "filedata\n");
}
