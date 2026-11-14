//! rubash#3349 — after `pwd -P` fails in a deleted cwd, default `pwd`
//! and `pwd -L` keep printing the stale logical string with rc 0.
//!
//! GNU builtins/cd.def pwd_builtin: when the physical face fails it
//! calls resetpwd ("pwd") (cd.def:183-192), which FREEs
//! the_current_working_directory and leaves it NULL; a later logical
//! face then takes the `get_working_directory ("pwd")` branch
//! (builtins/common.c:592), whose getcwd() fails again — diagnostic
//! plus rc 1. The physical failure poisons the cached logical value;
//! the logical face must not resurrect it.
//!
//! Expectations measured against GNU bash 5.3.20 (script files,
//! 2026-11-14; probes in ~/workspace/fix-3349/probes/gnu-chain.sh
//! and gnu-chain2.sh), stdout and stderr separately. Unix-only.
//! Control: a logical `pwd` FIRST (no failed -P before it) still
//! prints the stale value with rc 0 — GNU does the same, pinned by
//! #2798; only the failed physical call poisons.

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
fn issue3349_pwd_chain_after_failed_p_stays_failed() {
    let tree = make_tree("chain");
    let script = format!(
        "cd \"{r}/d\"; rmdir ../d; pwd -P; echo \"rc=$?\"; pwd; echo \"rc=$?\"; pwd -L; echo \"rc=$?\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\nrc=1\nrc=1\n");
    assert_eq!(
        stderr,
        format!("{GETCWD_DIAGNOSTIC}{GETCWD_DIAGNOSTIC}{GETCWD_DIAGNOSTIC}")
    );
}

#[test]
fn issue3349_pwd_chain_poison_is_session_local_to_subshell() {
    // A failed `pwd -P` inside a subshell poisons the SUBSHELL's
    // logical value (its own later `pwd` fails too) but must not
    // poison the parent: a subshell is a fork (GNU cd.def
    // change_to_directory / shell.c fork), and the parent's tcwd
    // stands.
    let tree = make_tree("subshell");
    let script = format!(
        "cd \"{r}/d\"; rmdir ../d; ( pwd -P; pwd; echo \"rc=$?\" ); pwd; echo \"rc=$?\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, format!("rc=1\n{r}/d\nrc=0\n", r = tree.real));
    assert_eq!(
        stderr,
        format!("{GETCWD_DIAGNOSTIC}{GETCWD_DIAGNOSTIC}")
    );
}

#[test]
fn issue3349_pwd_p_failure_in_comsub_does_not_poison_parent() {
    // `$(pwd -P)` is a fork: the failure (rc 1, one diagnostic)
    // stays inside the substitution and the parent's logical value
    // still prints stale with rc 0 afterwards.
    let tree = make_tree("comsub");
    let script = format!(
        "cd \"{r}/d\"; rmdir ../d; x=$(pwd -P); echo \"rc=$?\"; pwd; echo \"rc=$?\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, format!("rc=1\n{r}/d\nrc=0\n", r = tree.real));
    assert_eq!(stderr, GETCWD_DIAGNOSTIC);
}

#[test]
fn issue3349_pwd_chain_inside_comsub_is_poisoned() {
    // Inside one multi-command substitution the poisoning is live:
    // `pwd -P` fails, the later `pwd` in the SAME substitution
    // fails too — while the parent afterwards is unaffected.
    let tree = make_tree("comsubchain");
    let script = format!(
        "cd \"{r}/d\"; rmdir ../d; x=$(pwd -P; pwd; echo \"rc=$?\"); echo \"captured=[$x]\"; pwd; echo \"rc=$?\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        format!("captured=[rc=1]\n{r}/d\nrc=0\n", r = tree.real)
    );
    assert_eq!(
        stderr,
        format!("{GETCWD_DIAGNOSTIC}{GETCWD_DIAGNOSTIC}")
    );
}

#[test]
fn issue3349_control_pwd_chain_in_live_cwd() {
    // A failed -P never happens here, so no poisoning: every face
    // prints the live directory with rc 0.
    let tree = make_tree("live");
    let script = format!(
        "cd \"{r}/real\"; pwd -P; echo \"rc=$?\"; pwd; echo \"rc=$?\"; pwd -L; echo \"rc=$?\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(
        stdout,
        format!("{r}/real\nrc=0\n{r}/real\nrc=0\n{r}/real\nrc=0\n", r = tree.real)
    );
}

#[test]
fn issue3349_pwd_after_failed_p_recovers_on_cd() {
    // Poisoning is not sticky past a successful cd: cd re-seeds
    // the logical value (GNU cd.def change_to_directory / setpwd)
    // and the faces print the new directory again.
    //
    // Only stdout is asserted here: GNU's cd also prints a
    // `chdir: error retrieving current directory: getcwd: ...`
    // diagnostic on the way out of the poisoned state
    // (change_to_directory calls get_working_directory ("chdir")
    // when tcwd is NULL), which is a cd face, not this issue's.
    let tree = make_tree("recover");
    let script = format!(
        "cd \"{r}/d\"; rmdir ../d; pwd -P; pwd; cd \"{r}/real\"; pwd; echo \"rc=$?\"; pwd -P; echo \"rc=$?\"\n",
        r = tree.real
    );
    let (stdout, _stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        format!("{r}/real\nrc=0\n{r}/real\nrc=0\n", r = tree.real)
    );
}
