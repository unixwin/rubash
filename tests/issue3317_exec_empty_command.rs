//! Regression pins for rubash#3317 — `exec` with an empty command word
//! exits 126 ("Is a directory") instead of GNU's 127 ("not found").
//!
//! GNU ownership: builtins/exec.def:155-168 — when the search fails
//! (command == 0), exec_builtin checks `file_isdir (args[0])` on the
//! operand as written; anything that is not a directory earns
//! sh_notfound / EX_NOTFOUND (127). general.c:812-817 `file_isdir` is
//! `stat (fn) == 0 && S_ISDIR`, and `stat ("")` fails ENOENT, so the
//! empty word is not a directory — it is "not found". Rubash's
//! as-written probe joined the empty word onto the session cwd, which
//! yields the cwd itself (a directory) and misclassified it as 126.
//!
//! All expectations are the GNU bash 5.3.20 outputs for the same probe
//! scripts under LC_ALL=C.UTF-8.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3317-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

struct Outcome {
    stdout: String,
    stderr: String,
    rc: i32,
}

/// Run one probe script as `case.sh` with the fixture as cwd.
fn run(root: &Path, body: &str) -> Outcome {
    std::fs::write(root.join("case.sh"), body).expect("case script");
    let out = Command::new(BIN)
        .arg("case.sh")
        .current_dir(root)
        .env("LC_ALL", "C.UTF-8")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    Outcome {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        rc: out.status.code().unwrap_or(-1),
    }
}

#[test]
fn exec_empty_double_quoted_is_not_found_127() {
    let root = scratch("dq");
    let out = run(&root, "exec \"\"\n");
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr, "case.sh: line 1: exec: : not found\n");
    assert_eq!(out.rc, 127);
}

#[test]
fn exec_empty_single_quoted_is_not_found_127() {
    let root = scratch("sq");
    let out = run(&root, "exec ''\n");
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr, "case.sh: line 1: exec: : not found\n");
    assert_eq!(out.rc, 127);
}

#[test]
fn exec_empty_with_extra_args_is_not_found_127() {
    let root = scratch("args");
    let out = run(&root, "exec \"\" foo bar\n");
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr, "case.sh: line 1: exec: : not found\n");
    assert_eq!(out.rc, 127);
}

#[test]
fn exec_empty_in_subshell_is_not_found_127() {
    let root = scratch("sub");
    let out = run(&root, "(exec \"\")\n");
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr, "case.sh: line 1: exec: : not found\n");
    assert_eq!(out.rc, 127);
}

#[test]
fn exec_empty_in_function_is_not_found_127() {
    let root = scratch("fn");
    let out = run(&root, "f() { exec \"\"; }\nf\n");
    assert_eq!(out.stdout, "");
    // GNU attributes a function-body error to the definition line
    // (line 1), not the call line — verified against bash 5.3.9.
    assert_eq!(out.stderr, "case.sh: line 1: exec: : not found\n");
    assert_eq!(out.rc, 127);
}

#[test]
fn exec_empty_with_execfail_continues_with_127() {
    let root = scratch("execfail");
    let out = run(
        &root,
        "shopt -s execfail\nexec \"\"\necho \"after rc=$?\"\n",
    );
    assert_eq!(out.stdout, "after rc=127\n");
    assert_eq!(out.stderr, "case.sh: line 2: exec: : not found\n");
    assert_eq!(out.rc, 0);
}

#[test]
fn exec_genuine_nonexistent_path_stays_not_found_127() {
    let root = scratch("missing");
    let out = run(&root, "exec /nonexistent-xyz\n");
    assert_eq!(out.stdout, "");
    assert_eq!(
        out.stderr,
        "case.sh: line 1: /nonexistent-xyz: No such file or directory\n"
    );
    assert_eq!(out.rc, 127);
}

#[test]
fn exec_dot_stays_cannot_execute_126() {
    let root = scratch("dot");
    let out = run(&root, "exec .\n");
    assert_eq!(out.stdout, "");
    assert_eq!(
        out.stderr,
        "case.sh: line 1: exec: .: cannot execute: Is a directory\n"
    );
    assert_eq!(out.rc, 126);
}
