//! rubash#3348 — after $PWD is forged (the variable desynchronized
//! from the real cwd), `cd` / `pwd -L` inside a command substitution
//! still derive the logical value from the VARIABLE.
//!
//! GNU keeps the logical cwd as its own state,
//! `the_current_working_directory` (builtins/common.c:589): `pwd -L`
//! prints it verbatim (builtins/cd.def pwd_builtin), `cd` builds the
//! new value from it with make_absolute (cd.def change_to_directory),
//! and a user assignment to $PWD moves only the variable, never the
//! logical value. A forged $PWD therefore cannot move the logical cwd
//! inside `$( )` either — the substitution child runs on the cloned
//! tcwd state (subst.c:7143 command_substitute forks; the child
//! inherits the parent's tcwd, and startup rebinds an imported $PWD
//! only when it names the same directory, variables.c:907 set_pwd).
//!
//! The comsub fast paths bypassed the cloned state and re-derived
//! from env_vars["PWD"]: the `$(pwd)` word slot echoed the variable
//! verbatim (and the empty string after `unset PWD`), and the
//! `$(cd X && pwd)` / `$(cd X; pwd)` shortcut built its logical
//! display from validated_logical_pwd — $PWD re-derived, which falls
//! back to the PHYSICAL cwd (losing the symlink spelling) the moment
//! the forged value names no directory. The multi-command body that
//! reaches the real executor was already correct (D3 control).
//!
//! Expectations measured against GNU bash 5.3.20 (script files,
//! 2026-11-11). Unix-only: the shapes spell POSIX paths.

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
    let base = std::env::temp_dir().join(format!("rubash3348-{tag}-{}", std::process::id()));
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
fn issue3348_comsub_pwd_ignores_dot_suffix_forgery() {
    let tree = make_tree("dot");
    let script = format!(
        "cd \"{r}/link\"; PWD=\"$PWD/.\"; echo \"$(pwd)\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/link\n", r = tree.real));
}

#[test]
fn issue3348_comsub_pwd_l_ignores_dot_suffix_forgery() {
    let tree = make_tree("dot-l");
    let script = format!(
        "cd \"{r}/link\"; PWD=\"$PWD/.\"; echo \"$(pwd -L)\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/link\n", r = tree.real));
}

#[test]
fn issue3348_comsub_pwd_ignores_bogus_pwd() {
    let tree = make_tree("bogus");
    let script = format!(
        "cd \"{r}/link\"; PWD=/bogus; echo \"$(pwd)\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/link\n", r = tree.real));
}

#[test]
fn issue3348_comsub_pwd_l_ignores_bogus_pwd() {
    let tree = make_tree("bogus-l");
    let script = format!(
        "cd \"{r}/link\"; PWD=/bogus; echo \"$(pwd -L)\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/link\n", r = tree.real));
}

#[test]
fn issue3348_comsub_cd_pwd_ignores_dot_suffix_forgery() {
    let tree = make_tree("cd-dot");
    let script = format!(
        "cd \"{r}/link\"; PWD=\"$PWD/.\"; echo \"$(cd sub && pwd)\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/link/sub\n", r = tree.real));
}

#[test]
fn issue3348_comsub_cd_pwd_ignores_bogus_pwd() {
    let tree = make_tree("cd-bogus");
    let script = format!(
        "cd \"{r}/link\"; PWD=/bogus; echo \"$(cd sub && pwd)\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/link/sub\n", r = tree.real));
}

#[test]
fn issue3348_comsub_cd_semicolon_pwd_ignores_bogus_pwd() {
    let tree = make_tree("cd-semi");
    let script = format!(
        "cd \"{r}/link\"; PWD=/bogus; echo \"$(cd sub; pwd)\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/link/sub\n", r = tree.real));
}

#[test]
fn issue3348_comsub_pwd_ignores_dotdot_forgery() {
    let tree = make_tree("dotdot");
    let script = format!(
        "cd \"{r}/link/sub\"; PWD=\"$PWD/../sub\"; echo \"$(pwd)\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/link/sub\n", r = tree.real));
}

#[test]
fn issue3348_comsub_pwd_after_unset_pwd() {
    let tree = make_tree("unset");
    let script = format!(
        "cd \"{r}/link\"; unset PWD; echo \"$(pwd)\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/link\n", r = tree.real));
}

#[test]
fn issue3348_comsub_pwd_p_ignores_forgery() {
    let tree = make_tree("phys");
    let script = format!(
        "cd \"{r}/link\"; PWD=\"$PWD/.\"; echo \"$(pwd -P)\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/real\n", r = tree.real));
}

#[test]
fn issue3348_comsub_pwd_ignores_trailing_slash_forgery() {
    let tree = make_tree("slash");
    let script = format!(
        "cd \"{r}/link\"; PWD=\"$PWD/\"; echo \"$(pwd)\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/link\n", r = tree.real));
}

#[test]
fn issue3348_comsub_cd_pwd_p_ignores_forgery() {
    let tree = make_tree("cd-phys");
    let script = format!(
        "cd \"{r}/link\"; PWD=\"$PWD/.\"; echo \"$(cd sub && pwd -P)\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/real/sub\n", r = tree.real));
}

#[test]
fn issue3348_comsub_cd_dotdot_pwd_ignores_bogus_pwd() {
    let tree = make_tree("cd-dotdot");
    let script = format!(
        "cd \"{r}/link\"; PWD=/bogus; echo \"$(cd .. && pwd)\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}\n", r = tree.real));
}

#[test]
fn issue3348_comsub_pwd_invalid_option_reports_usage() {
    let tree = make_tree("badopt");
    let script = format!(
        "cd \"{r}/link\"; x=$(pwd -x); echo \"status=$? out=[$x]\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "status=2 out=[]\n");
    assert!(stderr.contains("pwd: -x: invalid option"), "{stderr}");
    assert!(stderr.contains("pwd: usage: pwd [-LP]"), "{stderr}");
}

#[test]
fn issue3348_comsub_pwd_invalid_option_under_bogus_pwd() {
    let tree = make_tree("badopt-bogus");
    let script = format!(
        "cd \"{r}/link\"; PWD=/bogus; x=$(pwd -x); echo \"status=$? out=[$x]\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "status=2 out=[]\n");
    assert!(stderr.contains("pwd: -x: invalid option"), "{stderr}");
    assert!(stderr.contains("pwd: usage: pwd [-LP]"), "{stderr}");
}

#[test]
fn issue3348_comsub_pwd_honours_physical_option() {
    let tree = make_tree("set-p");
    let script = format!(
        "cd \"{r}/link\"; set -P; echo \"$(pwd)\"; set +P\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, format!("{r}/real\n", r = tree.real));
}

#[test]
fn issue3348_comsub_pwd_help_word_prints_long_help() {
    // The fast path bypasses the simple-command funnel's shared
    // --help gate (rubash#2753 CHECK_HELPOPT), so it replays the
    // gate: GNU prints the long help into the capture with status 2.
    let tree = make_tree("help");
    let script = format!(
        "cd \"{r}/link\"; x=$(pwd --help); s=$?; echo \"$x\"; echo \"status=$s\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert!(stdout.starts_with("pwd: pwd [-LP]\n"), "{stdout}");
    assert!(
        stdout.contains("print the value of $PWD if it names the current working"),
        "{stdout}"
    );
    assert!(stdout.ends_with("status=2\n"), "{stdout}");
}

#[test]
fn issue3348_comsub_multi_command_body_ignores_dot_suffix_forgery() {
    let tree = make_tree("multi");
    let script = format!(
        "cd \"{r}/link\"; PWD=\"$PWD/.\"; x=$(cd sub; pwd; pwd -L); echo \"$x\"\n",
        r = tree.real
    );
    let (stdout, stderr, code) = run_script_in(&tree.base, &script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(
        stdout,
        format!("{r}/link/sub\n{r}/link/sub\n", r = tree.real)
    );
}
