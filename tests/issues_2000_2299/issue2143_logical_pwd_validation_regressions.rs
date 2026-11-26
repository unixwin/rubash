//! rubash#2143 — the logical $PWD was never validated against the
//! current directory: a forged or stale $PWD was trusted verbatim by
//! `pwd` / `pwd -L` and by every `cd` that builds its destination on
//! it. GNU keeps a validated logical cwd
//! (the_current_working_directory): variables.c set_pwd adopts an
//! imported $PWD only when it names "." (same_file), builtins/cd.def
//! change_to_directory builds destinations from that validated value,
//! bindpwd rebinds $PWD from it (while OLDPWD records the RAW old
//! variable), and builtins/common.c get_working_directory is what
//! `pwd` prints. A bare assignment to $PWD changes only the variable —
//! validation happens at the pwd/cd use points, never at assignment.
//!
//! Expectations measured against GNU bash 5.3.20 (script files,
//! 2026-10-30). Unix-only: the shapes spell POSIX paths; the Windows
//! persona's $PWD forms are the #537/#659 faces.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

/// A prepared tree: <base>/sub plus a symlink <base>/link -> <base>/real.
struct Tree {
    base: PathBuf,
    /// Canonical spelling of base (what physical lookups report).
    real: String,
}

fn make_tree(tag: &str) -> Tree {
    let base = std::env::temp_dir().join(format!("rubash2143-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("sub")).expect("create sub");
    std::fs::create_dir_all(base.join("real")).expect("create real");
    std::fs::create_dir_all(base.join("a")).expect("create a");
    std::fs::create_dir_all(base.join("b")).expect("create b");
    std::os::unix::fs::symlink(base.join("real"), base.join("link")).expect("create symlink");
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

fn run_script_in(dir: &std::path::Path, script: &str) -> (String, Option<i32>) {
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
        output.status.code(),
    )
}

#[test]
fn issue2143_forged_pwd_cd_dot_recomputes_and_oldpwd_keeps_raw_value() {
    let tree = make_tree("cddot");
    let (stdout, code) = run_script_in(
        &tree.base,
        "PWD=/nonexistent-xyz; cd .; echo rc=$?; pwd; echo \"PWD=$PWD\"; echo \"OLDPWD=$OLDPWD\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        format!(
            "rc=0\n{real}\nPWD={real}\nOLDPWD=/nonexistent-xyz\n",
            real = tree.real
        )
    );
}

#[test]
fn issue2143_forged_pwd_bare_pwd_recomputes_but_variable_is_not_rewritten() {
    let tree = make_tree("pwd");
    let (stdout, code) = run_script_in(
        &tree.base,
        "PWD=/nonexistent-xyz; echo \"var=[$PWD]\"; pwd; pwd -L; pwd -P; echo \"var=[$PWD]\"\n",
    );
    assert_eq!(code, Some(0));
    // GNU: the assignment alone leaves the variable forged, `pwd` and
    // `pwd -L` print the validated directory, and even after `pwd`
    // the variable itself is still forged (only `cd` rebinds it).
    assert_eq!(
        stdout,
        format!(
            "var=[/nonexistent-xyz]\n{real}\n{real}\n{real}\nvar=[/nonexistent-xyz]\n",
            real = tree.real
        )
    );
}

#[test]
fn issue2143_forged_pwd_cd_sub_builds_from_validated_base() {
    let tree = make_tree("cdsub");
    let (stdout, code) = run_script_in(
        &tree.base,
        "PWD=/nonexistent-xyz; cd sub; pwd; echo \"PWD=$PWD\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        format!("{real}/sub\nPWD={real}/sub\n", real = tree.real)
    );
}

#[test]
fn issue2143_sibling_pwd_cd_dot_recomputes() {
    let tree = make_tree("sibling");
    let script = format!(
        "cd \"{}\"; PWD=\"{}\"; cd .; pwd; echo \"PWD=$PWD\"\n",
        tree.base.join("a").display(),
        tree.base.join("b").display()
    );
    let (stdout, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        format!("{real}/a\nPWD={real}/a\n", real = tree.real)
    );
}

#[test]
fn issue2143_forged_pwd_cd_dotdot_recomputes() {
    let tree = make_tree("dotdot");
    let (stdout, code) = run_script_in(
        &tree.base.join("sub"),
        "PWD=/nonexistent-xyz; cd ..; pwd; echo \"PWD=$PWD\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, format!("{real}\nPWD={real}\n", real = tree.real));
}

#[test]
fn issue2143_relative_pwd_rejected_at_use_points() {
    let tree = make_tree("relative");
    let (stdout, code) = run_script_in(
        &tree.base,
        "PWD=relative-junk; pwd; cd .; echo \"PWD=$PWD\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, format!("{real}\nPWD={real}\n", real = tree.real));
}

#[test]
fn issue2143_valid_logical_pwd_through_symlink_survives() {
    let tree = make_tree("logical");
    // Control: a logical $PWD that DOES name the current directory
    // (reached through the symlink) is preserved verbatim — validation
    // must not collapse logical paths to physical ones.
    let (stdout, code) = run_script_in(
        &tree.base,
        "cd link; pwd; echo \"PWD=$PWD\"; cd .; pwd; echo \"PWD=$PWD\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        format!(
            "{real}/link\nPWD={real}/link\n{real}/link\nPWD={real}/link\n",
            real = tree.real
        )
    );
}

#[test]
fn issue2143_forged_env_pwd_replaced_at_startup() {
    let tree = make_tree("startup");
    let case = tree.base.join("case.sh");
    std::fs::write(&case, "pwd; echo \"PWD=$PWD\"\n").expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&tree.base)
        .arg(&case)
        .env("PWD", "/nonexistent-xyz")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_file(&case);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("{real}\nPWD={real}\n", real = tree.real)
    );
}
