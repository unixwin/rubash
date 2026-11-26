//! rubash#1480 — the standalone binary ignored `--` in its own
//! option parsing: `rubash -- -c 'echo nope'` treated -c as the
//! command string instead of as the script FILE operand. GNU: after
//! `--`, the next argument is the script file; "-c" does not exist
//! as a file, so it fails with "-c: No such file or directory",
//! rc=127. (The niubash host handled it — drift.)
//!
//! Probe: clusters/x3/repro/1480.sh.
//! Expectations verified against GNU bash 5.3.20.

use std::process::Command;

#[test]
fn double_dash_ends_shell_options() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["--", "-c", "echo nope"])
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(stdout, "");
    assert_eq!(output.status.code(), Some(127));
    assert!(
        stderr.contains("-c: No such file or directory"),
        "stderr: {stderr}"
    );
}
