//! rubash#3349 — after `pwd -P` fails in a deleted cwd, the default
//! `pwd` / `pwd -L` faces still print the stale logical string with
//! rc 0. GNU turns the same faces red: the failed `-P` goes through
//! resetpwd (builtins/cd.def:183-191), which frees
//! the_current_working_directory and leaves it NULL, so the next
//! logical face has no cached value to print and re-derives through
//! get_working_directory (builtins/common.c:592), whose getcwd()
//! fails the same way.
//!
//! GNU builtins/cd.def pwd_builtin (cd.def:457-523): the physical arm
//! is `sh_physpath (tcwd, 0)`; when it fails (`directory == 0`) the
//! `tcwd && directory == 0` arm calls `resetpwd ("pwd")`, which
//! NULLs tcwd and calls get_working_directory — the getcwd failure
//! prints the diagnostic and pwd returns EXECUTION_FAILURE. A later
//! logical face then takes the `tcwd == 0` arm:
//! `get_working_directory ("pwd")` again, same diagnostic, same
//! failure. The poison is the NULL, not the diagnostic.
//!
//! Expectations measured against GNU bash 5.3.20 (script files,
//! 2026-11-11), stdout and stderr separately. Unix-only.

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
    let base = std::env::temp_dir().join(format!("rubash3349-{tag}-{}", std::process::id()));
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
fn issue3349_chain_pwd_p_failure_poisons_logical_pwd() {
    let tree = make_tree("cell1");
    let script = format!(
        "cd \"{r}/d\"; rmdir ../d; pwd -P; echo \"rc=$?\"; pwd; echo \"rc=$?\"; pwd -L; echo \"rc=$?\"; pwd -P; echo \"rc=$?\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\nrc=1\nrc=1\nrc=1\n");
    assert_eq!(stderr, GETCWD_DIAGNOSTIC.repeat(4));
}

#[test]
fn issue3349_control_logical_pwd_alone_still_prints_stale() {
    // No physical failure has happened, so the cached logical value
    // stands: GNU prints it with rc 0 (the #2798 control, preserved).
    // Only the resetpwd poison turns these faces red.
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
fn issue3349_control_pwd_p_then_pwd_in_normal_cwd() {
    let tree = make_tree("ctrl2");
    let script = format!("pwd -P; echo \"rc=$?\"; pwd; echo \"rc=$?\"; pwd -L; echo \"rc=$?\"\n");
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(
        stdout,
        format!("{r}\nrc=0\n{r}\nrc=0\n{r}\nrc=0\n", r = tree.real)
    );
}

#[test]
fn issue3349_logical_first_then_pwd_p_failure_then_logical() {
    // -L alone does not poison; the -P after it does, and the pwd
    // after that is red.
    let tree = make_tree("cell2");
    let script = format!(
        "cd \"{r}/d\"; rmdir ../d; pwd -L; echo \"rc=$?\"; pwd -P; echo \"rc=$?\"; pwd; echo \"rc=$?\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, format!("{r}/d\nrc=0\nrc=1\nrc=1\n", r = tree.real));
    assert_eq!(stderr, GETCWD_DIAGNOSTIC.repeat(2));
}

#[test]
fn issue3349_subshell_pwd_p_failure_does_not_poison_parent() {
    // The subshell is a fork in GNU: the poison lives in the child's
    // the_current_working_directory only. The parent's cached logical
    // value still stands after the subshell exits.
    let tree = make_tree("cell3");
    let script = format!(
        "cd \"{r}/d\"; rmdir ../d; ( pwd -P; echo \"in_rc=$?\"; pwd; echo \"in_rc=$?\"; pwd -L; echo \"in_rc=$?\" ); pwd; echo \"rc=$?\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        format!("in_rc=1\nin_rc=1\nin_rc=1\n{r}/d\nrc=0\n", r = tree.real)
    );
    assert_eq!(stderr, GETCWD_DIAGNOSTIC.repeat(3));
}

#[test]
fn issue3349_set_p_default_physical_failure_poisons_too() {
    // With `set -P` the default mode IS physical (builtins/cd.def
    // pwd_builtin: verbatim_pwd = no_symbolic_links), so a bare pwd
    // takes the same sh_physpath -> resetpwd path and poisons the
    // cache; `set +P` afterwards still finds it NULL.
    let tree = make_tree("cell4");
    let script = format!(
        "cd \"{r}/d\"; rmdir ../d; set -P; pwd; echo \"rc=$?\"; set +P; pwd; echo \"rc=$?\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\nrc=1\n");
    assert_eq!(stderr, GETCWD_DIAGNOSTIC.repeat(2));
}

#[test]
fn issue3349_cd_out_after_poison_restores_logical_pwd() {
    // The poison is not permanent: a successful cd re-establishes
    // the_current_working_directory (builtins/cd.def
    // change_to_directory: set_working_directory), so pwd works again.
    // GNU's stderr here carries one more line than rubash's: the cd
    // entry re-derivation (cd.def:529-534 get_working_directory
    // ("chdir")) prints `chdir: error retrieving current directory:
    // getcwd: ...` before the move stands. That is cd's face of the
    // poisoned state, not pwd's, and is owed separately — this cell
    // pins only the pwd faces (two pwd diagnostics) plus the restore.
    let tree = make_tree("cell5");
    let script = format!(
        "cd \"{r}/d\"; rmdir ../d; pwd -P; echo \"rc=$?\"; pwd; echo \"rc=$?\"; cd \"{r}\"; pwd; echo \"rc=$?\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, format!("rc=1\nrc=1\n{r}\nrc=0\n", r = tree.real));
    assert_eq!(stderr, GETCWD_DIAGNOSTIC.repeat(2));
}
