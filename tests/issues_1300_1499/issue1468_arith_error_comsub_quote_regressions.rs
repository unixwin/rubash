//! rubash#1468 — an arithmetic error inside a command substitution
//! inside double quotes was misreported as an EOF parse error and the
//! whole line was dropped. GNU reports the real arithmetic error,
//! assigns the empty result, and the rest of the line runs (rc=1 for
//! the assignment).
//!
//! Probe: clusters/x2/repro/1468.sh.
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1468-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn arith_error_in_quoted_comsub_reports_real_error() {
    let dir = tempdir("arith");
    let script = dir.join("probe.sh");
    std::fs::write(
        &script,
        "a=$( echo \"q:$((1/0))\" ); echo \"got=[$a] rc=$?\"\n\
         printf 'E:%s\\n' \"${u:-$(printf \"it's\")}\"\n\
         echo \"second rc=$?\"\n\
         b=$( echo \"q:$((1%0))\" ); echo \"got2=[$b] rc=$?\"\n",
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
    assert_eq!(stdout, "got=[] rc=1\nE:it's\nsecond rc=0\ngot2=[] rc=1\n");
    assert!(
        stderr.contains("1/0: division by 0 (error token is \"0\")"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("1%0: division by 0 (error token is \"0\")"),
        "stderr: {stderr}"
    );
    assert!(
        !stderr.contains("unexpected EOF"),
        "must not be misreported as EOF, stderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
