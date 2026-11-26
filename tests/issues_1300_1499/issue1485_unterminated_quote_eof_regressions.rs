//! rubash#1485 — an unterminated-quote EOF diagnostic was repeated
//! once per following line; GNU reports it exactly once and the
//! script exits 2.
//!
//! Probe: clusters/x3/repro/1485.sh (the quote opened on line 2 is
//! never closed; three assignment lines follow).
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1485-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn unterminated_quote_eof_reported_once() {
    let dir = tempdir("eof");
    let script = dir.join("probe.sh");
    std::fs::write(
        &script,
        "echo \"start\"\ne10='quote\\'dq\"'\ne11='hex\\x4'\ne12='fine'\necho \"end\"\n",
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
    assert_eq!(stdout, "start\n");
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        stderr
            .matches("unexpected EOF while looking for matching")
            .count(),
        1,
        "diagnostic must appear exactly once, stderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
