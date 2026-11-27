//! rubash#1412 — `builtin exec` with redirections only wrongly made
//! them PERSISTENT; GNU treats a redir-only `exec` reached via
//! `builtin` as a one-shot no-op (files are created by the redirect
//! itself, but no fd assignment survives).
//!
//! Probes: clusters/k9/probes/p1412{,b,c}.sh.
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1412-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run_script(dir: &PathBuf, text: &str) -> (String, String, Option<i32>) {
    let script = dir.join("probe.sh");
    std::fs::write(&script, text).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn builtin_exec_stdout_redir_is_one_shot() {
    let dir = tempdir("stdout");
    let (stdout, _stderr, code) =
        run_script(&dir, "builtin exec >f; echo AFTER; echo \"f=[$(cat f)]\"\n");
    assert_eq!(stdout, "AFTER\nf=[]\n");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn builtin_exec_fd_redir_does_not_persist() {
    let dir = tempdir("fd");
    let (stdout, stderr, code) = run_script(
        &dir,
        "builtin exec 3>f; echo hi >&3; echo AFTER; echo \"f=[$(cat f)]\"\n",
    );
    assert_eq!(stdout, "AFTER\nf=[]\n");
    assert!(
        stderr.contains("3: Bad file descriptor"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn builtin_exec_stderr_redir_is_one_shot() {
    let dir = tempdir("stderr");
    let (stdout, stderr, code) = run_script(
        &dir,
        "builtin exec 2>e; echo hi; ls /nonexistent_xyz_1; echo \"e=[$(cat e)]\"\n",
    );
    assert_eq!(stdout, "hi\ne=[]\n");
    assert!(
        stderr.contains("cannot access '/nonexistent_xyz_1'"),
        "ls diagnostic must stay on the real stderr, stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}
