//! rubash#3575 — POSIX `same_file` arm missing: under `set -o posix`,
//! `pwd -L` in a deleted cwd still prints the stale logical path.
//! GNU builtins/cd.def pwd_builtin (cd.def:457-523): after computing
//! `directory`, the arm
//!   (posixly_correct && same_file (".", tcwd, (struct stat *)0, (struct stat *)0) == 0)
//! calls resetpwd ("pwd"), which NULLs the_current_working_directory
//! and re-derives via get_working_directory — whose getcwd() fails in
//! a deleted cwd, printing the diagnostic with rc 1
//! (builtins/common.c:592-624). same_file (general.c:673-692) stats
//! both sides and compares (dev, ino); either stat failing trips it.
//!
//! Expectations measured against GNU bash 5.2.21 (script files,
//! 2026-11-16), stdout and stderr separately. Unix-only.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

/// Tree: <base>/d, plus <base>/real and <base>/link -> real for controls.
struct Tree {
    base: PathBuf,
    /// Canonical spelling of base (what physical lookups report).
    real: String,
}

fn make_tree(tag: &str) -> Tree {
    let base = std::env::temp_dir().join(format!("rubash3575-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("d")).expect("create d");
    std::fs::create_dir_all(base.join("real")).expect("create real");
    std::os::unix::fs::symlink("real", base.join("link")).expect("create link");
    let real = std::fs::canonicalize(&base)
        .expect("canonicalize base")
        .to_string_lossy()
        .into_owned();
    Tree { base, real }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn run_script_in(dir: &std::path::Path, script: &str) -> (String, String, Option<i32>) {
    run_script_in_with_env(dir, script, &[])
}

fn run_script_in_with_env(
    dir: &std::path::Path,
    script: &str,
    extra_env: &[(&str, &str)],
) -> (String, String, Option<i32>) {
    let case = dir.join("case.sh");
    std::fs::write(&case, script).expect("write fixture");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.current_dir(dir).arg(&case);
    for (key, value) in extra_env {
        cmd.env(key, value);
    }
    let output = cmd
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

const GETCWD_DIAGNOSTIC: &str =
    "pwd: error retrieving current directory: getcwd: cannot access parent directories: No such file or directory\n";

#[test]
fn issue3575_posix_pwd_l_in_deleted_cwd_fails() {
    // The core face: under `set -o posix`, the FIRST pwd call in a
    // deleted cwd is already `pwd -L` — no earlier `-P` failure has
    // poisoned the cache (#3349's chain does not apply). GNU's
    // same_file arm re-derives through resetpwd and fails the same
    // way on every face.
    let tree = make_tree("cell1");
    let script = format!(
        "set -o posix; cd \"{r}/d\"; rmdir ../d; pwd -L; echo \"rc=$?\"; pwd; echo \"rc=$?\"; pwd -P; echo \"rc=$?\"; pwd -L; echo \"rc=$?\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\nrc=1\nrc=1\nrc=1\n");
    assert_eq!(stderr, GETCWD_DIAGNOSTIC.repeat(4));
}

#[test]
fn issue3575_nonposix_control_still_prints_stale() {
    // The arm is POSIX-gated: without `set -o posix` the cached
    // logical value still stands and prints with rc 0 (the #2798 /
    // #3349 control, preserved verbatim).
    let tree = make_tree("ctrl1");
    let script = format!(
        "cd \"{r}/d\"; rmdir ../d; pwd; echo \"rc=$?\"; pwd -L; echo \"rc=$?\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/d\nrc=0\n{r}/d\nrc=0\n", r = tree.real));
}

#[test]
fn issue3575_posix_live_cwd_unchanged() {
    // same_file passes when the cwd is alive, so POSIX mode prints
    // exactly as before — including through a symlinked spelling,
    // where the stat comparison follows the link to the same inode.
    let tree = make_tree("ctrl2");
    let script = format!(
        "set -o posix; cd \"{r}/d\"; pwd -L; echo \"rc=$?\"; pwd; echo \"rc=$?\"; pwd -P; echo \"rc=$?\"; cd \"{r}/link\"; pwd -L; echo \"rc=$?\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(
        stdout,
        format!(
            "{r}/d\nrc=0\n{r}/d\nrc=0\n{r}/d\nrc=0\n{r}/link\nrc=0\n",
            r = tree.real
        )
    );
}

#[test]
fn issue3575_posix_subshell_failure_does_not_spare_parent() {
    // The arm is a live per-call check, not cache poison: the
    // subshell's resetpwd cannot touch the parent, yet the parent's
    // own `pwd -L` / `pwd` fail too — each call re-checks same_file.
    // (GNU probe P9: in_rc=1, rc=1, rc=1, three diagnostics.)
    let tree = make_tree("cell2");
    let script = format!(
        "set -o posix; cd \"{r}/d\"; rmdir ../d; ( pwd -L; echo \"in_rc=$?\" ); pwd -L; echo \"rc=$?\"; pwd; echo \"rc=$?\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "in_rc=1\nrc=1\nrc=1\n");
    assert_eq!(stderr, GETCWD_DIAGNOSTIC.repeat(3));
}

#[test]
fn issue3575_posix_poison_survives_set_plus_o_posix() {
    // The arm's resetpwd NULLs the cached value, so the failure
    // outlives the mode flip: after `set +o posix` the faces take
    // the `tcwd == 0` arm (get_working_directory) and fail the same
    // way. (GNU probe P4.)
    let tree = make_tree("cell3");
    let script = format!(
        "set -o posix; cd \"{r}/d\"; rmdir ../d; pwd -L; echo \"rc=$?\"; set +o posix; pwd -L; echo \"rc=$?\"; pwd; echo \"rc=$?\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\nrc=1\nrc=1\n");
    assert_eq!(stderr, GETCWD_DIAGNOSTIC.repeat(3));
}

#[test]
fn issue3575_posix_cd_out_after_failure_restores_pwd() {
    // The poison is not permanent: a successful cd re-establishes
    // the_current_working_directory, so pwd works again.
    // GNU's stderr here carries one more line than rubash's: the cd
    // entry re-derivation (cd.def:529-534 get_working_directory
    // ("chdir")) prints `chdir: error retrieving current directory:
    // getcwd: ...` before the move stands. That is cd's face of the
    // poisoned state, not pwd's, and is owed separately (#3375 owed
    // item 1, recorded in #3349 cell5) — this cell pins only the pwd
    // faces (one pwd diagnostic) plus the restore.
    let tree = make_tree("cell4");
    let script = format!(
        "set -o posix; cd \"{r}/d\"; rmdir ../d; pwd -L; echo \"rc=$?\"; cd \"{r}\"; pwd; echo \"rc=$?\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, format!("rc=1\n{r}\nrc=0\n", r = tree.real));
    assert_eq!(stderr, GETCWD_DIAGNOSTIC);
}

#[test]
fn issue3575_posixly_correct_env_enables_arm() {
    // GNU variables.c:5820 + 6257: an IMPORTED POSIXLY_CORRECT —
    // present with any value, even empty — turns posix mode on at
    // startup, so the arm fires with no `set -o posix` in the
    // script. (GNU probe P7.)
    let tree = make_tree("cell5");
    let script = format!(
        "cd \"{r}/d\"; rmdir ../d; pwd -L; echo \"rc=$?\"; pwd; echo \"rc=$?\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) =
        run_script_in_with_env(&tree.base, &script, &[("POSIXLY_CORRECT", "y")]);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\nrc=1\n");
    assert_eq!(stderr, GETCWD_DIAGNOSTIC.repeat(2));
}

#[test]
fn issue3575_posix_pwd_variable_stays_stale() {
    // resetpwd frees the_current_working_directory but never touches
    // $PWD: after the failed `pwd -L` the variable still holds the
    // stale spelling. (GNU probe P1.)
    let tree = make_tree("cell6");
    let script = format!(
        "set -o posix; cd \"{r}/d\"; rmdir ../d; pwd -L; echo \"rc=$?\"; echo \"PWD=$PWD\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, format!("rc=1\nPWD={r}/d\n", r = tree.real));
    assert_eq!(stderr, GETCWD_DIAGNOSTIC);
}
