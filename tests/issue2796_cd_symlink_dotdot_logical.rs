//! rubash#2796 — after `cd` into a symlinked directory, `cd ..`
//! (default and explicit `-L`) resolves the `..` PHYSICALLY: the shell
//! actually lands in the physical parent, while $PWD is rebound by
//! logical truncation, so `pwd -L` / `pwd -P` print the physical parent
//! and the variable disagrees with the shell's own position.
//!
//! GNU keeps ONE current-working-directory value
//! (builtins/common.c:589 the_current_working_directory):
//! builtins/cd.def change_to_directory makes the operand absolute
//! against that LOGICAL value (make_absolute) and, in logical mode,
//! chdirs to sh_canonpath's textual canonicalization
//! (`.`/`..` processed on the logical spelling, symlinks NOT resolved
//! first); only `-P` uses sh_physpath (symlinks resolved before `..`).
//! bindpwd (cd.def bindpwd) then binds $PWD from the same value, so
//! the landing spot, `pwd -L`, `pwd -P` and $PWD can never desync the
//! way they do here.
//!
//! Expectations measured against GNU bash 5.3.20 (script files,
//! 2026-11-09). Unix-only: the shapes spell POSIX paths.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

/// Tree A: <base>/a/real (+sub), <base>/b (+other), <base>/b/link -> ../a/real
/// Tree C: <base>/c/a/real, <base>/c/link -> a/real
struct Tree {
    base: PathBuf,
    /// Canonical spelling of base (what physical lookups report).
    real: String,
}

fn make_tree(tag: &str) -> Tree {
    let base = std::env::temp_dir().join(format!("rubash2796-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("a/real/sub")).expect("create a/real/sub");
    std::fs::create_dir_all(base.join("b/other")).expect("create b/other");
    std::os::unix::fs::symlink("../a/real", base.join("b/link")).expect("create b/link");
    std::fs::create_dir_all(base.join("c/a/real")).expect("create c/a/real");
    std::os::unix::fs::symlink("a/real", base.join("c/link")).expect("create c/link");
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

#[test]
fn issue2796_cd_dotdot_from_symlink_dir_lands_on_logical_parent() {
    let tree = make_tree("dotdot");
    let script = format!(
        "cd \"{r}/b/link\"; cd ..; pwd -L; pwd -P; echo \"PWD=$PWD\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/b\n{r}/b\nPWD={r}/b\n", r = tree.real));
}

#[test]
fn issue2796_cd_dash_l_dotdot_from_symlink_dir_lands_on_logical_parent() {
    let tree = make_tree("dashl");
    let script = format!(
        "cd \"{r}/b/link\"; cd -L ..; pwd -L; pwd -P; echo \"PWD=$PWD\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/b\n{r}/b\nPWD={r}/b\n", r = tree.real));
}

#[test]
fn issue2796_cd_dotdot_other_from_symlink_dir_uses_logical_sibling() {
    let tree = make_tree("other");
    let script = format!(
        "cd \"{r}/b/link\"; cd ../other; echo \"rc=$?\"; pwd -L; echo \"PWD=$PWD\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(
        stdout,
        format!("rc=0\n{r}/b/other\nPWD={r}/b/other\n", r = tree.real)
    );
}

#[test]
fn issue2796_cd_dotdot_from_deleted_cwd_reaches_parent() {
    let tree = make_tree("deleted");
    std::fs::create_dir_all(tree.base.join("x")).expect("create x");
    let script = format!(
        "cd \"{r}/x\"; rmdir ../x; cd ..; echo \"rc=$?\"; pwd -L; echo \"PWD=$PWD\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("rc=0\n{r}\nPWD={r}\n", r = tree.real));
}

#[test]
fn issue2796_chain_cd_link_dotdot_then_link_again() {
    let tree = make_tree("chain");
    let script = format!(
        "cd \"{r}/c\"; cd link; cd ..; pwd; cd link; pwd -P\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/c\n{r}/c/a/real\n", r = tree.real));
}

#[test]
fn issue2796_control_cd_dash_p_dotdot_uses_physical_parent() {
    let tree = make_tree("ctrlp");
    let script = format!(
        "cd \"{r}/b/link\"; cd -P ..; pwd -L; pwd -P; echo \"PWD=$PWD\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/a\n{r}/a\nPWD={r}/a\n", r = tree.real));
}

#[test]
fn issue2796_control_cd_dotdot_dotdot_converges() {
    let tree = make_tree("ctrldd");
    let script = format!(
        "cd \"{r}/b/link\"; cd ../..; pwd -L; pwd -P; echo \"PWD=$PWD\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}\n{r}\nPWD={r}\n", r = tree.real));
}

#[test]
fn issue2796_control_cd_subdir_of_symlink_then_dotdot() {
    let tree = make_tree("ctrlsub");
    let script = format!(
        "cd \"{r}/b/link/sub\"; cd ..; pwd -L; pwd -P; echo \"PWD=$PWD\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(
        stdout,
        format!("{r}/b/link\n{r}/a/real\nPWD={r}/b/link\n", r = tree.real)
    );
}

#[test]
fn issue2796_control_cd_dot_in_symlink_dir_stays() {
    let tree = make_tree("ctrldot");
    let script = format!(
        "cd \"{r}/b/link\"; cd .; pwd -L; pwd -P; echo \"PWD=$PWD\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(
        stdout,
        format!("{r}/b/link\n{r}/a/real\nPWD={r}/b/link\n", r = tree.real)
    );
}
