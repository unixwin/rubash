//! Regression pins for rubash#3314 — the `exec` builtin face
//! reported a bad interpreter at 127 instead of 126 when the `exec`
//! ran at the top level of a `-c` string.
//!
//! The issue's seed re-invokes the shell under test with `-c`
//! (`"$BASH" -c 'exec ./bad.sh'`), because the divergence is
//! top-level only: inside a `( )` subshell rubash already reported
//! 126. The script-file top-level shape is already pinned by the
//! #3148 stem (`exec_bad_interpreter_top_level_is_sys_error_126`);
//! this stem pins the `-c` face the #3314 seed exercises, plus the
//! subshell control and the two not-found controls (mG1/mG2) from
//! the issue, which must stay 127.
//!
//! GNU ownership: `exec` with a command operand resolves the target
//! (builtins/exec.def:151) and hands it to shell_execve
//! (execute_cmd.c:6126). A failed execve on an executable `#!` file
//! is classified at execute_cmd.c:6184 through
//! `sys_error ("%s: %s: %s", command, interp, "bad interpreter")`
//! for ANY errno (the sample check is errno-agnostic) and returns
//! EX_NOEXEC (126). sys_error is error.c:275 -> error_prolog (0):
//! the prolog is get_name_for_error alone — no line segment. The
//! sibling not-found classifications go through file_error /
//! builtin_error (error_prolog (1)) and keep the `line 1:` segment.
//! failed_exec (exec.def:255-258) exits a non-interactive shell —
//! and a subshell — at that status, so the top level and the
//! subshell agree at 126.
//!
//! The `-c` prolog is the shell name as invoked (get_name_for_error's
//! $0 arm, error.c:90-120): the test binary is invoked by its full
//! path, so the prolog is the `CARGO_BIN_EXE_rubash` path itself —
//! the same rule GNU follows (the oracle prints its own full
//! invocation path as the prolog). The report name is the
//! exec.def:172 full_pathname form (general.c:931 — cwd join with
//! MP_RMDOT), i.e. the absolute script path.
//!
//! All expectations are the GNU bash 5.3.20 outputs for the same
//! probes under LC_ALL=C.UTF-8, measured against the oracle binary.
//! NOTE: bash 5.2.21 answers this shape differently (127 with
//! "cannot execute: required file not found") — the bad-interpreter
//! sys_error classification is the 5.3 behaviour, and 5.3.x is the
//! contract version; do not "verify" these pins against a 5.2 bash.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3314-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    // The report name is built from the shell's (physical) cwd, so the
    // fixture root is canonicalized once and used in that form for the
    // child cwd and every expectation.
    std::fs::canonicalize(&dir).expect("canonical root")
}

struct Outcome {
    stdout: String,
    stderr: String,
    rc: i32,
}

/// Run the shell under test with `-c` (the #3314 seed shape):
/// the top-level exec context. `BASH_ARGV0` is removed from the
/// child environment so the `-c` prolog derives from the invoked
/// binary path alone, never from an inherited import (main.rs
/// re-seeds $0 from an imported BASH_ARGV0 verbatim).
fn run_c(root: &Path, script: &str) -> Outcome {
    let out = Command::new(BIN)
        .arg("-c")
        .arg(script)
        .current_dir(root)
        .env("LC_ALL", "C.UTF-8")
        .env_remove("BASH_ARGV0")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash -c");
    Outcome {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        rc: out.status.code().unwrap_or(-1),
    }
}

fn fixture(tag: &str) -> PathBuf {
    let root = scratch(tag);
    let path = root.join("bad.sh");
    std::fs::write(&path, "#!/nonexistent-interp-xyz\necho body-line\n").expect("fixture file");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("fixture mode");
    root
}

fn bad_interpreter_line(root: &Path) -> String {
    format!(
        "{BIN}: {}/bad.sh: /nonexistent-interp-xyz: bad interpreter: No such file or directory\n",
        root.display()
    )
}

#[test]
fn exec_bad_interpreter_dash_c_top_level_is_sys_error_126() {
    let root = fixture("toplevel");
    let out = run_c(&root, "exec ./bad.sh");
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr, bad_interpreter_line(&root));
    assert_eq!(out.rc, 126);
}

#[test]
fn exec_bad_interpreter_dash_c_in_subshell_is_126() {
    let root = fixture("subshell");
    let out = run_c(&root, "( exec ./bad.sh ); echo \"rc=$?\"");
    assert_eq!(out.stdout, "rc=126\n");
    assert_eq!(out.stderr, bad_interpreter_line(&root));
    assert_eq!(out.rc, 0);
}

#[test]
fn exec_missing_slash_name_dash_c_keeps_line_segment_and_127() {
    let root = fixture("missing");
    let out = run_c(&root, "exec ./nonexistent.sh");
    assert_eq!(out.stdout, "");
    assert_eq!(
        out.stderr,
        format!(
            "{BIN}: line 1: {}/nonexistent.sh: No such file or directory\n",
            root.display()
        )
    );
    assert_eq!(out.rc, 127);
}

#[test]
fn exec_absolute_missing_dash_c_is_127() {
    let root = fixture("absent");
    let out = run_c(&root, "exec /nonexistent-xyz");
    assert_eq!(out.stdout, "");
    assert_eq!(
        out.stderr,
        format!("{BIN}: line 1: /nonexistent-xyz: No such file or directory\n")
    );
    assert_eq!(out.rc, 127);
}
