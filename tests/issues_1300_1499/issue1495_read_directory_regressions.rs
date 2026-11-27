//! rubash#1495 — `read` from a directory failed silently; GNU
//! diagnoses "read: 0: read error: Is a directory" with rc=1 (a read
//! from /dev/null is the silent rc=1 control).
//!
//! Probe: clusters/x4/repro/1495.sh.
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1495-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn read_from_directory_diagnoses() {
    let dir = tempdir("readdir");
    std::fs::create_dir_all(dir.join("d")).expect("create subdir");
    let script = dir.join("probe.sh");
    std::fs::write(
        &script,
        "read x < $PWD/d; echo \"rc=$?\"\nread y < /dev/null; echo \"rc2=$?\"\n",
    )
    .expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(stdout, "rc=1\nrc2=1\n");
    assert!(
        stderr.contains("read: 0: read error: Is a directory"),
        "stderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
