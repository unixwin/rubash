//! Builtin diagnostics inside a pipeline component must print in
//! execution order, ahead of the same component's later stdout
//! (rubash#3434; GNU bash 5.3.20 oracle-verified 2026-11-13).
//!
//! GNU `execute_pipeline` (execute_cmd.c:2620) forks each element with
//! the pipe bound to fd 1 and the shell's other descriptors inherited,
//! and `builtin_error` writes the element's fd 2 as produced
//! (builtins/common.c:98, unbuffered; shell.c:1921-1922 line-buffers
//! stdio), so in a merged stream the diagnostic for the earlier
//! command precedes the stdout of the later one. Rubash ran a
//! compound/function stage with its whole stderr captured into one
//! buffer and emitted only when the stage completed, while the
//! stage's stdout streamed live (final member, #1605) or escaped
//! live through a dup'd fd (`>&3`), so the error that execution
//! order puts first printed last. Expansion errors were NOT deferred
//! (they bypass the capture through the fd-2 diagnostic path), so
//! they jumped ahead of the earlier builtin error. The fix arms the
//! #2483 live-stderr channel in the sequential stage loop whenever
//! the stage's fd 2 is live (see `pipeline_stage_stderr_may_stream`).
//!
//! Harness: the shell under test runs as a child (`-c`) with its
//! stdout and stderr bound to the SAME file by the harness (one
//! shared open file description, so both streams share one offset
//! and the file records the child's true emission order). The merge
//! is invisible to the child — no in-shell `2>&1` — which is exactly
//! the context that failed: an in-shell dup already routes the
//! diagnostic correctly (guarded by the last test). Needles are
//! substrings, so the binary-path diagnostic prefix stays stable.
//! `LC_ALL=C.UTF-8` is set on the child so the pin is hermetic.

use std::fs::{self, File};
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn rubash_bin() -> String {
    env!("CARGO_BIN_EXE_rubash").to_string()
}

struct MergedRun {
    merged: String,
    code: i32,
    dir: PathBuf,
}

/// Run `script` in a child rubash with stdout+stderr on one shared
/// file; return the merged bytes in emission order.
fn merged_child_output(tag: &str, script: &str) -> MergedRun {
    let dir = std::env::temp_dir().join(format!("issue3434-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("merged.txt");
    let stdout_file = File::create(&path).expect("create merged file");
    let stderr_file = stdout_file.try_clone().expect("clone merged file");
    let status = Command::new(rubash_bin())
        .args(["-c", script])
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file))
        .status()
        .expect("spawn rubash");
    MergedRun {
        merged: fs::read_to_string(&path).expect("read merged file"),
        code: status.code().unwrap_or(-1),
        dir,
    }
}

/// Assert `first` appears before `second` in the merged stream.
fn assert_before(tag: &str, merged: &str, first: &str, second: &str) {
    let a = merged.find(first);
    let b = merged.find(second);
    assert!(
        matches!((a, b), (Some(a), Some(b)) if a < b),
        "{tag}: [{first}] must precede [{second}] in merged output:\n{merged}"
    );
}

/// Assert three needles appear in order in the merged stream.
fn assert_order3(tag: &str, merged: &str, first: &str, second: &str, third: &str) {
    assert_before(tag, merged, first, second);
    assert_before(tag, merged, second, third);
}

const READ_DIAG: &str = "not a valid identifier";
const CD_DIAG: &str = "/nonexistent-xyz";
const EXPANSION_DIAG: &str = "bad substitution";
const AFTER: &str = "after=1";

/// Seed check 1: right side of a pipe, `{ }` group, one builtin
/// error then stdout.
#[test]
fn issue3434_pipe_right_brace_diag_before_stdout() {
    let run = merged_child_output("brace", "printf \"v\\n\" | { read -- -n; echo after=$?; }");
    assert_before("brace", &run.merged, READ_DIAG, AFTER);
}

/// Seed check 2: right side of a pipe, `( )` subshell.
#[test]
fn issue3434_pipe_right_paren_diag_before_stdout() {
    let run = merged_child_output("paren", "printf \"v\\n\" | ( read -- -n; echo after=$?; )");
    assert_before("paren", &run.merged, READ_DIAG, AFTER);
}

/// Seed check 3: two builtin errors then stdout — relative order
/// among the errors is preserved, and both precede the stdout.
#[test]
fn issue3434_pipe_right_two_errors_before_stdout() {
    let run = merged_child_output(
        "two-err",
        "printf \"v\\n\" | { read -- -n; cd /nonexistent-xyz; echo after=$?; }",
    );
    assert_order3("two-err", &run.merged, READ_DIAG, CD_DIAG, AFTER);
}

/// Seed check 4: left side of a pipe, stdout on a direct fd.
#[test]
fn issue3434_pipe_left_direct_fd_diag_before_stdout() {
    let run = merged_child_output(
        "left-direct",
        "{ read -- -n; echo after=$? >&3; } 3>&1 | cat",
    );
    assert_before("left-direct", &run.merged, READ_DIAG, AFTER);
}

/// Morph mQ3: an expansion error must not jump ahead of the
/// earlier builtin error.
#[test]
fn issue3434_pipe_expansion_error_after_builtin_error() {
    let run = merged_child_output(
        "exp-after-cd",
        "printf \"v\\n\" | { cd /nonexistent-xyz; echo \"${-x}\"; echo after=$?; }",
    );
    assert_before("exp-after-cd", &run.merged, CD_DIAG, EXPANSION_DIAG);
}

/// Morph mR7: the read error precedes the later expansion error.
#[test]
fn issue3434_pipe_expansion_error_after_read_error() {
    let run = merged_child_output(
        "exp-after-read",
        "printf \"v\\n\" | { read -- -n; echo \"${-x}\"; echo after=$?; }",
    );
    assert_before("exp-after-read", &run.merged, READ_DIAG, EXPANSION_DIAG);
}

/// Function stage: the same live-stderr arming covers a function
/// element's diagnostics.
#[test]
fn issue3434_pipe_function_stage_diag_before_stdout() {
    let run = merged_child_output(
        "function",
        "f() { read -- -n; echo after=$?; }; printf \"v\\n\" | f",
    );
    assert_before("function", &run.merged, READ_DIAG, AFTER);
}

/// Control: plain sequence, no pipeline — already agreed on base.
#[test]
fn issue3434_control_plain_sequence() {
    let run = merged_child_output("control-seq", "read -- -n <<< \"v\"; echo after=$?");
    assert_before("control-seq", &run.merged, READ_DIAG, AFTER);
}

/// Control: command substitution, no pipeline — already agreed.
#[test]
fn issue3434_control_command_substitution() {
    let run = merged_child_output(
        "control-comsub",
        "x=$(read -- -n; echo after=$?;); echo \"got:[$x]\"",
    );
    assert_before("control-comsub", &run.merged, READ_DIAG, "got:[after=1]");
}

/// Guard: a stage's own `2>file` still claims its diagnostics —
/// they must land in the file, never on the live stderr — while the
/// stage's stdout stays on the merged stream.
#[test]
fn issue3434_stage_stderr_redirect_still_wins() {
    let dir = std::env::temp_dir().join(format!("issue3434-stderr-file-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    let err_path = dir.join("err.txt");
    let script = format!(
        "printf \"v\\n\" | {{ read -- -n; echo after=$?; }} 2>{}",
        err_path.to_string_lossy()
    );
    let run = merged_child_output("stderr-file", &script);
    assert!(
        run.merged.contains(AFTER),
        "stderr-file: merged stream must carry the stdout line:\n{}",
        run.merged
    );
    assert!(
        !run.merged.contains(READ_DIAG),
        "stderr-file: the diagnostic must not escape to the merged stream:\n{}",
        run.merged
    );
    let err = fs::read_to_string(&err_path).expect("read err file");
    assert!(
        err.contains(READ_DIAG),
        "stderr-file: the diagnostic must land in the redirect target:\n{err}"
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&run.dir);
}

/// Guard: the group's own `2>&1` merge (applied in-shell) keeps
/// production order — the in-stage dup path the fix must not disturb.
#[test]
fn issue3434_control_group_merge_in_shell() {
    let run = merged_child_output(
        "group-merge",
        "printf \"v\\n\" | { read -- -n; echo after=$?; } 2>&1",
    );
    assert_before("group-merge", &run.merged, READ_DIAG, AFTER);
}
