//! rubash#2798 — with the cwd deleted, `pwd -P` silently prints the
//! stale directory and returns 0 instead of reporting the getcwd
//! failure.
//!
//! GNU builtins/cd.def pwd_builtin in physical mode runs
//! sh_physpath on the_current_working_directory; when that fails it
//! resets through get_working_directory (builtins/common.c:592),
//! whose getcwd() call fails for a deleted cwd and prints
//! `pwd: error retrieving current directory: getcwd: cannot access
//! parent directories: No such file or directory` (for_whom is the
//! literal caller name, common.c:611) — pwd then returns
//! EXECUTION_FAILURE. Logical faces never touch getcwd while the
//! logical value stands, so default `pwd` / `pwd -L` keep printing
//! $PWD with rc 0 in the same state (the control cells).
//!
//! Expectations measured against GNU bash 5.3.20 (script files,
//! 2026-11-09), stdout and stderr separately. Unix-only.

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
    let base = std::env::temp_dir().join(format!("rubash2798-{tag}-{}", std::process::id()));
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

const GETCWD_DIAGNOSTIC: &str =
    "pwd: error retrieving current directory: getcwd: cannot access parent directories: No such file or directory\n";

#[test]
fn issue2798_pwd_p_in_deleted_cwd_reports_getcwd_failure() {
    let tree = make_tree("cell1");
    let script = format!(
        "cd \"{r}/d\"; rmdir ../d; pwd -P; echo \"rc=$?\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(stderr, GETCWD_DIAGNOSTIC);
}

#[test]
fn issue2798_pwd_p_in_deleted_cwd_subshell_reports_getcwd_failure() {
    let tree = make_tree("cell2");
    let script = format!(
        "cd \"{r}/d\"; rmdir ../d; ( pwd -P; echo \"rc=$?\" )\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(stderr, GETCWD_DIAGNOSTIC);
}

#[test]
fn issue2798_control_logical_pwd_in_deleted_cwd_prints_pwd() {
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
fn issue2798_control_pwd_p_in_normal_cwd() {
    let tree = make_tree("ctrl2");
    let (stdout, stderr, code) = run_script_in(&tree.base, "pwd -P; echo \"rc=$?\"\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}\nrc=0\n", r = tree.real));
}

#[test]
fn issue2798_control_pwd_p_through_symlink() {
    let tree = make_tree("ctrl3");
    let script = format!("cd \"{r}/link\"; pwd -P; echo \"rc=$?\"\n", r = tree.real);
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/real\nrc=0\n", r = tree.real));
}
