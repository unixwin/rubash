//! rubash#2799 — a child shell process discards the inherited
//! logical $PWD and resets it to the physical directory, so the
//! child's `pwd -L` and $PWD both lose the symlink spelling the
//! parent exported.
//!
//! GNU variables.c:907 set_pwd (run for every fresh shell) adopts an
//! imported $PWD when it is absolute (ABSPATH) and names "." —
//! same_file(temp_string, ".") — canonicalizing it with
//! sh_canonpath (CHECKDOTDOT|CHECKEXISTS: spelling canonicalization,
//! NO symlink resolution) into the_current_working_directory. Only a
//! $PWD that fails that test is replaced by getcwd(). The $PWD
//! variable itself keeps the imported spelling in non-POSIX mode.
//!
//! Expectations measured against GNU bash 5.3.20 (script files and
//! env-controlled spawns, 2026-11-09). Unix-only.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

/// Tree: <base>/real, <base>/link -> real
struct Tree {
    base: PathBuf,
    /// Canonical spelling of base (what physical lookups report).
    real: String,
}

fn make_tree(tag: &str) -> Tree {
    let base = std::env::temp_dir().join(format!("rubash2799-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
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

/// Spawn a fresh shell process directly, with the process cwd inside
/// the symlink directory and a controlled PWD in its environment —
/// the exact shape variables.c set_pwd adjudicates at startup.
fn run_child_with_env(cwd: &std::path::Path, pwd: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(cwd)
        .env("PWD", pwd)
        .arg("-c")
        .arg("pwd -L; pwd -P; echo \"PWD=$PWD\"")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run child rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue2799_child_process_inherits_logical_pwd() {
    let tree = make_tree("child");
    let script = format!(
        "cd \"{r}/link\"; \"{bin}\" -c 'pwd -L; pwd -P; echo \"PWD=$PWD\"'\n",
        r = tree.real,
        bin = env!("CARGO_BIN_EXE_rubash")
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(
        stdout,
        format!("{r}/link\n{r}/real\nPWD={r}/link\n", r = tree.real)
    );
}

#[test]
fn issue2799_child_process_with_explicit_pwd_prefix() {
    let tree = make_tree("prefix");
    let script = format!(
        "cd \"{r}/link\"; PWD=\"$PWD\" \"{bin}\" -c 'pwd -L; pwd -P'\n",
        r = tree.real,
        bin = env!("CARGO_BIN_EXE_rubash")
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/link\n{r}/real\n", r = tree.real));
}

#[test]
fn issue2799_child_adopts_noncanonical_logical_pwd() {
    let tree = make_tree("noncanon");
    // The imported spelling is adopted as the logical directory in
    // canonicalized form (pwd -L), while the $PWD variable itself
    // keeps the imported spelling verbatim.
    let (stdout, stderr, code) =
        run_child_with_env(&tree.base.join("link"), &format!("{}/link/.", tree.real));
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(
        stdout,
        format!("{r}/link\n{r}/real\nPWD={r}/link/.\n", r = tree.real)
    );
}

#[test]
fn issue2799_control_child_rejects_foreign_pwd() {
    let tree = make_tree("foreign");
    let (stdout, stderr, code) = run_child_with_env(&tree.base.join("link"), "/nonexistent-xyz");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(
        stdout,
        format!("{r}/real\n{r}/real\nPWD={r}/real\n", r = tree.real)
    );
}

#[test]
fn issue2799_control_parent_exports_logical_pwd() {
    let tree = make_tree("printenv");
    let script = format!("cd \"{r}/link\"; printenv PWD\n", r = tree.real);
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/link\n", r = tree.real));
}

#[test]
fn issue2799_control_child_after_cd_dash_p() {
    let tree = make_tree("dashp");
    let script = format!(
        "cd -P \"{r}/link\"; \"{bin}\" -c 'pwd -L; pwd -P; echo \"PWD=$PWD\"'\n",
        r = tree.real,
        bin = env!("CARGO_BIN_EXE_rubash")
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(
        stdout,
        format!("{r}/real\n{r}/real\nPWD={r}/real\n", r = tree.real)
    );
}

#[test]
fn issue2799_control_subshell_keeps_logical_pwd() {
    let tree = make_tree("subshell");
    let script = format!(
        "cd \"{r}/link\"; ( pwd -L; echo \"PWD=$PWD\" )\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/link\nPWD={r}/link\n", r = tree.real));
}
