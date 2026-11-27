//! rubash#1482 — a command name containing a backslash was
//! diagnosed "No such file or directory"; GNU diagnoses it
//! "command not found" (rc=127), like any other unresolvable name
//! without a slash-driven path lookup failure.
//!
//! Probe: clusters/x3/repro/1482.sh.
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1482-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn backslash_command_name_is_command_not_found() {
    let dir = tempdir("backslash");
    let script = dir.join("probe.sh");
    std::fs::write(&script, "'a\\b'; echo \"rc=$?\"\n").expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(stdout, "rc=127\n");
    assert!(
        stderr.contains("a\\b: command not found"),
        "stderr: {stderr}"
    );
    assert!(
        !stderr.contains("No such file or directory"),
        "stderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
