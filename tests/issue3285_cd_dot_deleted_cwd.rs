//! rubash#3285 — `cd .` in a deleted cwd: the getcwd-failure diagnostic
//! and the `/.` PWD spelling (residual b4 of rubash#3268, from the
//! PR #3283 review probe `review-3283/probe-residuals.log`).
//!
//! GNU law: builtins/cd.def change_to_directory (cd.def:522) makes the
//! operand absolute against the_current_working_directory
//! (make_absolute), canonicalizes it, and — when canonicalization fails
//! because the cwd itself is unlinked — chdirs the operand VERBATIM:
//! a dot-only operand resolves through the cwd inode itself, so the
//! move stands. resetpwd ("cd") (cd.def:183) then fails getcwd and
//! builtins/common.c:592 get_working_directory prints
//! `cd: error retrieving current directory: getcwd: cannot access
//! parent directories: No such file or directory` (the middle phrase
//! is bash_getcwd_errstr, general.c:70) and returns NULL, so
//! set_working_directory (tdir) binds the uncanonicalized
//! make_absolute form — $PWD gains the `/.` suffix.
//!
//! The mechanism landed on master via PR #3386 (Fixes #3335, shared
//! `deleted_dot_move` in src/builtins/cd.rs); this pin is the
//! issue-#3285 green lock on the exact review-probe shape, with
//! fail-first evidence against the pre-#3386 base (178fbf8^): the
//! base prints no diagnostic and binds the bare `D/a` spelling.
//!
//! Expectations measured against GNU bash 5.3.20 (2026-11-13, local
//! oracle run of the review-probe shape). Unix-only: the shapes spell
//! POSIX paths.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

const GETCWD_DIAG: &str = "cd: error retrieving current directory: getcwd: cannot access parent directories: No such file or directory\n";

fn make_tree(tag: &str) -> (PathBuf, String) {
    let base = std::env::temp_dir().join(format!("rubash3285-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("a")).expect("create a");
    let real = std::fs::canonicalize(&base)
        .expect("canonicalize base")
        .to_string_lossy()
        .into_owned();
    (base, real)
}

fn run_script_in(dir: &std::path::Path, script: &str) -> (String, String, Option<i32>) {
    let case = dir.join("case.sh");
    std::fs::write(&case, script).expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(dir)
        .arg(&case)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_file(&case);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue3285_cd_dot_deleted_cwd_diag_and_pwd() {
    // The review-probe shape, verbatim
    // (review-3283/probe-residuals.sh, section "b4 cd ."):
    // enter a, delete it from a subshell, `cd .` — rc 0, the getcwd
    // diagnostic on stderr, and $PWD keeps the `/.` tail.
    let (base, real) = make_tree("cd-dot");
    let script = format!(
        "cd \"{r}/a\"; (cd ..; rmdir a); cd .; echo \"rc=$? PWD=$PWD\"\n",
        r = real
    );
    let (stdout, stderr, code) = run_script_in(&base, &script);
    let _ = std::fs::remove_dir_all(&base);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, GETCWD_DIAG);
    assert_eq!(stdout, format!("rc=0 PWD={r}/a/.\n", r = real));
}
