//! rubash#1481 — `fi extra`: the engine EXECUTED the if body before
//! reporting the syntax error; GNU parses the whole compound command
//! first, so on a trailing-junk error nothing in the body runs and
//! the script exits 2 (earlier complete commands already ran).
//!
//! Probe: clusters/x3/repro/1481.sh.
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1481-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn fi_extra_aborts_before_body_executes() {
    let dir = tempdir("fiextra");
    let script = dir.join("probe.sh");
    std::fs::write(&script, "echo FIRST\nif true; then\necho in-if\nfi extra\n")
        .expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(stdout, "FIRST\n", "the if body must not run: {stdout}");
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr.contains("syntax error near unexpected token `extra'"),
        "stderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
