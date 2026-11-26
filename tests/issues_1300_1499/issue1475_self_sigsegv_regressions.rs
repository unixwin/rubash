//! rubash#1475 — SIGSEGV sent to the shell itself was swallowed
//! (`kill -SEGV $$` had no effect; a subshell doing it reported
//! rc=0). GNU: the shell dies by the signal (139 at the top level;
//! in a subshell the subshell dies with a "Segmentation fault"
//! notice and rc=139). Self-SIGKILL/SIGTERM already agreed.
//!
//! Probe: clusters/x2/repro/1475a.sh, 1475b.sh.
//! Expectations verified against GNU bash 5.3.20 (script files).

#![cfg(unix)]

use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1475-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run_script(tag: &str, text: &str) -> (String, String, Option<i32>, Option<i32>) {
    let dir = tempdir(tag);
    let script = dir.join("probe.sh");
    std::fs::write(&script, text).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let result = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
        output.status.signal(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn self_sigsegv_kills_the_shell() {
    let (stdout, _stderr, code, signal) = run_script("top", "kill -SEGV $$; echo alive\n");
    assert_eq!(stdout, "");
    assert_eq!(code, None, "shell must die by the signal, not exit");
    assert_eq!(signal, Some(11), "SIGSEGV");
}

#[test]
fn subshell_self_sigsegv_dies_139_and_shell_survives_until_own_term() {
    let (stdout, stderr, code, signal) = run_script(
        "sub",
        "( kill -SEGV $BASHPID ); echo \"rc=$?\"\necho outer-alive\nkill -TERM $$; echo alive2\n",
    );
    assert_eq!(stdout, "rc=139\nouter-alive\n");
    assert!(stderr.contains("Segmentation fault"), "stderr: {stderr}");
    assert_eq!(code, None);
    assert_eq!(signal, Some(15), "the final self-SIGTERM ends the shell");
}
