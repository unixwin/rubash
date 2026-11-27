//! rubash#2775 — with `shopt varredir_close` OFF (the default), a
//! command-scoped `{var}` redirection must PERSIST after the command:
//! GNU registers the undo for a REDIR_VARASSIGN entry only
//! `if (varassign_redir_autoclose)` (redir.c:972, twins 1091/1153), so
//! with the option off no undo exists and the descriptor stays in the
//! shell's table, still named by the variable. With the option ON both
//! shells close it at command end. External commands keep the binding
//! child-only in both shells (the parent variable stays unset).
//!
//! Expectations verified against GNU bash 5.3.20 with identical
//! script files (fresh process, stdin null).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2775-{tag}-{}", std::process::id()));
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
fn issue2775_builtin_binding_persists_when_option_off() {
    let dir = tempdir("builtin");
    let (stdout, stderr, code) = run_script(
        &dir,
        "{v}>f.txt echo hi\necho \"v=$v\"\necho more >&$v\necho \"rc=$?\"\ncat f.txt\n",
    );
    assert_eq!(stdout, "hi\nv=10\nrc=0\nmore\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue2775_true_and_function_bindings_persist() {
    let dir = tempdir("fn");
    let (stdout, stderr, code) = run_script(
        &dir,
        "{v}>f.txt true\necho more >&$v\necho \"rc1=$?\"\nfn() { :; }\n{w}>g.txt fn\necho evenmore >&$w\necho \"rc2=$?\"\ncat f.txt g.txt\n",
    );
    assert_eq!(stdout, "rc1=0\nrc2=0\nmore\nevenmore\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue2775_compound_binding_persists() {
    let dir = tempdir("compound");
    let (stdout, stderr, code) = run_script(
        &dir,
        "{ echo hi; } {v}>f.txt\necho \"v=$v\"\necho more >&$v\necho \"rc=$?\"\ncat f.txt\n",
    );
    assert_eq!(stdout, "hi\nv=10\nrc=0\nmore\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue2775_loop_binding_persists() {
    let dir = tempdir("loop");
    let (stdout, stderr, code) = run_script(
        &dir,
        "n=0\nwhile [ $n -lt 1 ]; do n=1; done {v}>f.txt\necho more >&$v\necho \"rc=$?\"\n",
    );
    assert_eq!(stdout, "rc=0\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue2775_option_on_still_closes() {
    // Control: with varredir_close ON, the descriptor is closed at
    // command end in both shells — writing through the variable then
    // fails with a Bad file descriptor diagnostic.
    let dir = tempdir("on");
    let (stdout, stderr, code) = run_script(
        &dir,
        "shopt -s varredir_close\n{v}>f.txt echo hi\necho more >&$v\necho \"rc=$?\"\n",
    );
    assert_eq!(stdout, "hi\nrc=1\n", "stderr: {stderr}");
    assert!(
        stderr.contains("Bad file descriptor"),
        "stderr should carry the bad-fd diagnostic: {stderr}"
    );
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}
