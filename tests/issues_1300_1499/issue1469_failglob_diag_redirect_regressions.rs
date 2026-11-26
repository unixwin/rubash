//! rubash#1469 — the failglob "no match" diagnostic bypassed the
//! command's stderr redirection and escaped to the shell's outer
//! stderr; GNU routes it through the current execution context, so
//! `2>err.txt` and `2>&1` capture both see it.
//!
//! Probe: clusters/x2/repro/1469.sh.
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1469-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn failglob_diagnostic_honors_stderr_redirection() {
    let dir = tempdir("failglob");
    let script = dir.join("probe.sh");
    std::fs::write(
        &script,
        "shopt -s failglob\n\
         ( echo *.zzz_no ) 2>err.txt; echo \"err=[$(cat err.txt)]\"\n\
         cap=$( ( echo *.zzz_no ) 2>&1 ); echo \"cap=[$cap]\"\n\
         echo done\n",
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
    assert!(
        stdout.contains("err=[") && stdout.contains("no match: *.zzz_no]"),
        "diagnostic must land in err.txt, stdout: {stdout}"
    );
    assert!(
        stdout.contains("cap=[") && stdout.matches("no match: *.zzz_no").count() == 2,
        "diagnostic must be captured by 2>&1 too, stdout: {stdout}"
    );
    assert!(stdout.contains("done"), "stdout: {stdout}");
    assert_eq!(stderr, "", "nothing may leak to outer stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}
