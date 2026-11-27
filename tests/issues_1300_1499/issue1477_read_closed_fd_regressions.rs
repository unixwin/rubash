//! rubash#1477 — a FAILED `read` (closed stdin) still bound the
//! variable to empty; GNU leaves it unset and diagnoses
//! "read: 0: read error: Bad file descriptor", rc=1.
//!
//! Probe: clusters/x3/repro/1477.sh.
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1477-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn failed_read_leaves_variable_unset() {
    let dir = tempdir("closedfd");
    let script = dir.join("probe.sh");
    std::fs::write(&script, "read x <&-\necho \"rc=$? x=${x-UNSET}\"\n").expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(stdout, "rc=1 x=UNSET\n");
    assert!(
        stderr.contains("read: 0: read error: Bad file descriptor"),
        "stderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
