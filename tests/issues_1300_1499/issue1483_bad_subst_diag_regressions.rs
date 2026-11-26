//! rubash#1483 — bad-substitution diagnostics: the echoed word kept
//! a stray `$` and lost the enclosing word context. GNU renders the
//! offending expansion as written: `${'x'}: bad substitution` for
//! `echo ${$'x'}`, and the script continues with the next command.
//!
//! Probe: clusters/x3/repro/1483.sh.
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1483-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn bad_substitution_diagnostic_echoes_the_word() {
    let dir = tempdir("badsubst");
    let script = dir.join("probe.sh");
    std::fs::write(&script, "echo ${$'x'}\necho \"s1:[${s@}]\"\n").expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(stdout, "s1:[]\n");
    assert!(
        stderr.contains("${'x'}: bad substitution"),
        "stderr: {stderr}"
    );
    assert_eq!(output.status.code(), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}
