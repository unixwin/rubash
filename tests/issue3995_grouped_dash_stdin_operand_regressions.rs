//! rubash#3995: `cat -` (the explicit stdin operand) inside a
//! brace group / subshell / function whose stdin is redirected
//! from a file printed NOTHING (rc=0, silent), while the
//! no-operand `cat` in the same group and a `cat -` carrying its
//! own redirect both worked. The surviving copy in the original
//! repro (`{ cat -; } < f; cat < f` printing the file once) came
//! from the SECOND command; the grouped `cat -` itself was the
//! empty one. The same face broke `exec 0<f; cat -`.
//!
//! Root cause: the operand path decides streaming-vs-buffered via
//! cat_stdin_is_real_process_stdin, which PEEKED through
//! stdin_string_for_command. When fd 0 is an fd-table File
//! endpoint bound by an enclosing compound (or an exec rebind) —
//! the cat node itself carries no redirect — that text serve's
//! fd-table arm drains the endpoint's shared-offset stream to
//! EOF (read_fd_read_endpoint_bytes read_to_end). The peek ate
//! the bytes; the real serve in cat_stdin_operand_bytes then read
//! an exhausted stream. The fix classifies the source from
//! non-consuming facts (fd-0 redirect winner, closed flag,
//! FUNCTION_STDIN presence, in-memory endpoint snapshot) instead
//! of draining the stream it is about to serve.
//!
//! Oracle: GNU bash 5.3.20, script files, byte-compared
//! (stdout/stderr/status), cloud, 2026-11-21.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3995-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    std::fs::write(dir.join("f1"), "alpha\nbeta\n").expect("write f1");
    std::fs::write(dir.join("g1"), "gamma\n").expect("write g1");
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

/// The original repro: the grouped `cat -` must print the file,
/// and the second command re-reads it fresh.
#[test]
fn grouped_dash_then_second_read() {
    assert_run("{ cat -; } < f1; cat < f1", "alpha\nbeta\nalpha\nbeta\n");
}

/// The grouped serve must come from the group's own binding —
/// not from whatever a later command opens.
#[test]
fn grouped_dash_serves_group_file_not_later_file() {
    assert_run("{ cat -; } < f1; cat < g1", "alpha\nbeta\ngamma\n");
}

/// GNU shares the descriptor: the grouped `cat -` drains the
/// stream, so a following no-operand `cat` in the same group
/// sees EOF — total output is one copy, produced by `cat -`.
#[test]
fn grouped_dash_drains_for_following_plain_cat() {
    assert_run("{ cat -; cat; } < f1", "alpha\nbeta\n");
}

/// Control (already correct): the plain cat drains first, the
/// later `cat -` sees EOF.
#[test]
fn grouped_plain_then_dash_sees_eof() {
    assert_run("{ cat; cat -; } < f1", "alpha\nbeta\n");
}

/// After a grouped `cat -`, a `read` in the same group is at
/// EOF (the dash operand really consumed the shared stream).
#[test]
fn grouped_dash_drains_for_following_read() {
    assert_run(
        "{ cat -; read x; echo \"r:$x\"; } < f1",
        "alpha\nbeta\nr:\n",
    );
}

/// The subshell position of the same face.
#[test]
fn subshell_dash_then_second_read() {
    assert_run("( cat - ) < f1; cat < f1", "alpha\nbeta\nalpha\nbeta\n");
}

/// The function position of the same face.
#[test]
fn function_dash_then_second_read() {
    assert_run(
        "fn() { cat -; }; fn < f1; cat < f1",
        "alpha\nbeta\nalpha\nbeta\n",
    );
}

/// A later command's OWN redirect opens the file fresh — the
/// grouped drain must not poison it.
#[test]
fn grouped_dash_then_own_redirect_read_is_fresh() {
    assert_run(
        "{ cat -; } < f1; read x < f1; echo \"read:$x\"",
        "alpha\nbeta\nread:alpha\n",
    );
}

/// Operand-form second command, same freshness.
#[test]
fn grouped_dash_then_operand_cat() {
    assert_run("{ cat -; } < f1; cat f1", "alpha\nbeta\nalpha\nbeta\n");
}

/// Each group binds its own fd 0; both grouped dashes serve.
#[test]
fn two_dash_groups() {
    assert_run(
        "{ cat -; } < f1; { cat -; } < f1",
        "alpha\nbeta\nalpha\nbeta\n",
    );
}

/// `cat - -` in a group: the first operand drains, the second
/// sees EOF; the following command re-reads fresh.
#[test]
fn grouped_dash_twice_operands_then_second() {
    assert_run(
        "{ cat - -; } < f1; cat < f1",
        "alpha\nbeta\nalpha\nbeta\n",
    );
}

/// A later `cat -` with its OWN redirect was always fine and
/// must stay fine.
#[test]
fn grouped_dash_then_dash_with_own_redirect() {
    assert_run("{ cat -; } < f1; cat - < f1", "alpha\nbeta\nalpha\nbeta\n");
}

/// Same mechanism, exec-bind position: `exec 0<f` installs the
/// fd-table endpoint the peek used to drain.
#[test]
fn exec_bound_dash() {
    assert_run("exec 0<f1; cat -", "alpha\nbeta\n");
}

/// Control (already correct): `cat -` with its own redirect.
#[test]
fn own_redirect_dash_unchanged() {
    assert_run("cat - < f1; cat < f1", "alpha\nbeta\nalpha\nbeta\n");
}

/// Control (already correct): no-operand cat in a group.
#[test]
fn grouped_plain_cat_unchanged() {
    assert_run("{ cat; } < f1; cat < f1", "alpha\nbeta\nalpha\nbeta\n");
}

/// Guard: pipeline stages ride the FUNCTION_STDIN channel —
/// the classification must keep them on the buffered serve.
#[test]
fn pipeline_stage_group_dash() {
    assert_run("echo hi | { cat -; }", "hi\n");
}

/// Guard: a plain pipeline-stage `cat -` likewise.
#[test]
fn pipeline_stage_plain_dash() {
    assert_run("echo hi | cat -", "hi\n");
}
