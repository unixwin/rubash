//! rubash#1312 — `set -C` (noclobber) handling of existing targets:
//! GNU refuses to overwrite an existing REGULAR file
//! ("cannot overwrite existing file", rc=1, content preserved) but
//! device targets such as /dev/null are not regular files and the
//! redirect succeeds. (The Windows /dev/tty half of the issue is
//! covered by the Windows-gated device tests; this file pins the
//! platform-neutral core.)
//!
//! Probe: clusters/x6/probes/p1312.sh.
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1312-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn noclobber_refuses_regular_file_but_allows_device() {
    let dir = tempdir("noclobber");
    let script = dir.join("probe.sh");
    std::fs::write(
        &script,
        "set -C\necho hi >/dev/null; echo \"rc-null=$?\"\necho seed > regf\necho hi > regf; echo \"rc-reg=$?\"\ncat regf\n",
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
    assert_eq!(stdout, "rc-null=0\nrc-reg=1\nseed\n");
    assert!(
        stderr.contains("cannot overwrite existing file"),
        "stderr: {stderr}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("regf")).expect("regf"),
        "seed\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
