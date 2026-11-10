//! Regression pins for rubash#2661 — `exec` of a directory reported
//! "not found" / 127 instead of GNU's "cannot execute: Is a directory"
//! / 126.
//!
//! GNU ownership: builtins/exec.def:153-172 — when the search fails
//! (command == 0), exec_builtin checks `file_isdir (args[0])` on the
//! operand AS WRITTEN: a directory earns
//! `builtin_error ("%s: cannot execute: Is a directory")` with
//! EX_NOEXEC (126); anything else earns sh_notfound / EX_NOTFOUND
//! (127). The check is not gated on the operand containing a slash:
//! `exec .` and `exec somedir` are the directory forms. A directory
//! found through a PATH element is discarded by the walk itself
//! (findcmd.c FS_NODIRS) and the as-written name is not a directory
//! in the cwd, so it stays "not found" / 127.
//!
//! All expectations are the GNU bash 5.3.20 outputs for the same
//! probe scripts under LC_ALL=C.UTF-8.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-2661-{tag}-{}", std::process::id()));
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
        .env("FIX", root)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    Outcome {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        rc: out.status.code().unwrap_or(-1),
    }
}

fn fixture(tag: &str) -> PathBuf {
    let root = scratch(tag);
    std::fs::create_dir_all(root.join("somedir")).expect("somedir");
    std::fs::create_dir_all(root.join("pdir").join("dircmd")).expect("pdir/dircmd");
    root
}

#[test]
fn exec_dot_is_cannot_execute_126() {
    let root = fixture("dot");
    let out = run(&root, "exec .\n");
    assert_eq!(out.stdout, "");
    assert_eq!(
        out.stderr,
        "case.sh: line 1: exec: .: cannot execute: Is a directory\n"
    );
    assert_eq!(out.rc, 126);
}

#[test]
fn exec_bare_directory_name_is_cannot_execute_126() {
    let root = fixture("bare");
    let out = run(&root, "exec somedir\n");
    assert_eq!(out.stdout, "");
    assert_eq!(
        out.stderr,
        "case.sh: line 1: exec: somedir: cannot execute: Is a directory\n"
    );
    assert_eq!(out.rc, 126);
}

#[test]
fn exec_slash_directory_keeps_cannot_execute_126() {
    let root = fixture("slash");
    let out = run(&root, "exec ./somedir\n");
    assert_eq!(out.stdout, "");
    assert_eq!(
        out.stderr,
        "case.sh: line 1: exec: ./somedir: cannot execute: Is a directory\n"
    );
    assert_eq!(out.rc, 126);
}

#[test]
fn exec_missing_name_stays_not_found_127() {
    let root = fixture("missing");
    let out = run(&root, "exec nosuchcmd2661\n");
    assert_eq!(out.stdout, "");
    assert_eq!(
        out.stderr,
        "case.sh: line 1: exec: nosuchcmd2661: not found\n"
    );
    assert_eq!(out.rc, 127);
}

#[test]
fn exec_directory_through_path_element_stays_not_found_127() {
    let root = fixture("pathdir");
    let out = run(&root, "PATH=./pdir:/usr/bin:/bin\nexec dircmd\n");
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr, "case.sh: line 2: exec: dircmd: not found\n");
    assert_eq!(out.rc, 127);
}
