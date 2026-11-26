//! rubash#2797 — `pwd -L` echoes the $PWD VARIABLE verbatim when the
//! value is a valid but non-canonical spelling of the current
//! directory (`/.`, trailing `/`, `/../`, `//` components), and falls
//! back to the PHYSICAL directory when the variable does not name the
//! current directory at all.
//!
//! GNU `pwd -L` never prints the variable: builtins/cd.def
//! pwd_builtin prints the_current_working_directory
//! (builtins/common.c:589) — the shell's own logical cwd, established
//! by cd (change_to_directory's sh_canonpath canonicalization) and by
//! startup adoption (variables.c:907 set_pwd canonicalizes an adopted
//! $PWD with sh_canonpath). A bare assignment to $PWD changes only the
//! variable; the logical cwd — and therefore `pwd -L`'s output — is
//! unaffected, even by `unset PWD`. Distinct from closed #2143 (a
//! forged $PWD that does not name the current directory being trusted
//! at use points): here the value DOES name the current directory,
//! only its spelling is non-canonical, plus the no-valid-variable
//! fallback target itself.
//!
//! Expectations measured against GNU bash 5.3.20 (script files,
//! 2026-11-09). Unix-only: the shapes spell POSIX paths.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

/// Tree: <base>/real (+sub), <base>/link -> real
struct Tree {
    base: PathBuf,
    /// Canonical spelling of base (what physical lookups report).
    real: String,
}

fn make_tree(tag: &str) -> Tree {
    let base = std::env::temp_dir().join(format!("rubash2797-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("real/sub")).expect("create real/sub");
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

#[test]
fn issue2797_pwd_l_canonicalizes_dot_suffixed_pwd() {
    let tree = make_tree("dot");
    let script = format!("cd \"{r}/link\"; PWD=\"$PWD/.\" pwd -L\n", r = tree.real);
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/link\n", r = tree.real));
}

#[test]
fn issue2797_pwd_l_canonicalizes_trailing_slash_pwd() {
    let tree = make_tree("slash");
    let script = format!("cd \"{r}/link\"; PWD=\"$PWD/\" pwd -L\n", r = tree.real);
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/link\n", r = tree.real));
}

#[test]
fn issue2797_pwd_l_canonicalizes_dotdot_component_pwd() {
    let tree = make_tree("dotdot");
    let script = format!(
        "cd \"{r}/link/sub\"; PWD=\"$PWD/../sub\" pwd -L\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/link/sub\n", r = tree.real));
}

#[test]
fn issue2797_pwd_l_canonicalizes_double_slash_component_pwd() {
    let tree = make_tree("dblslash");
    let script = format!("cd \"{r}/link\"; PWD=\"$PWD//.\" pwd -L\n", r = tree.real);
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/link\n", r = tree.real));
}

#[test]
fn issue2797_pwd_l_ignores_bogus_pwd_assignment() {
    let tree = make_tree("bogus");
    let script = format!(
        "cd \"{r}/link\"; PWD=/bogus; echo \"var=$PWD\"; pwd -L\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("var=/bogus\n{r}/link\n", r = tree.real));
}

#[test]
fn issue2797_control_pwd_p_ignores_pwd_spelling() {
    let tree = make_tree("ctrlp");
    let script = format!("cd \"{r}/link\"; PWD=\"$PWD/.\" pwd -P\n", r = tree.real);
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/real\n", r = tree.real));
}

#[test]
fn issue2797_unset_pwd_keeps_logical_directory() {
    let tree = make_tree("unset");
    let script = format!("cd \"{r}/link\"; unset PWD; pwd -L; pwd\n", r = tree.real);
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/link\n{r}/link\n", r = tree.real));
}
