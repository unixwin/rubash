//! rubash#1490 — printf with an unterminated %( format errored
//! (rc=1); GNU prints the partial format literally with a warning
//! and returns 0. A bare invalid conversion char (%Y outside %())
//! still fails that printf (rc=1 via the following echo's $?).
//!
//! Probe: clusters/x4/repro/1490.sh.
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1490-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn printf_unterminated_time_format_prints_literally() {
    let dir = tempdir("unterm");
    let script = dir.join("probe.sh");
    std::fs::write(
        &script,
        "printf 'F:%(\\n' 0; echo \"rc=$?\"\n\
         printf 'G:%(abc' 0; echo \"rc2=$?\"\n\
         printf 'H:%Y\\n' 0; echo \"rc3=$?\"\n",
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
    assert_eq!(stdout, "F:%(\nrc=0\nG:%(abcrc2=0\nH:rc3=1\n");
    assert_eq!(
        stderr.matches("invalid time format specification").count(),
        2,
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("`Y': invalid format character"),
        "stderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
