//! rubash#1476 — `lastpipe` still applied under `set -m`; GNU treats
//! lastpipe as inert while job control (monitor mode) is on: the
//! last pipeline element runs in a subshell and assignments are lost.
//! With monitor off, lastpipe applies normally.
//!
//! Probe: clusters/x2/repro/1476.sh.
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn lastpipe_is_inert_under_monitor_mode() {
    let (stdout, stderr, code) = rubash(
        "x=before\n\
         set -m\n\
         shopt -s lastpipe\n\
         echo hello | read x\n\
         echo \"monitor-on: x=$x\"\n\
         set +m\n\
         echo world | read x\n\
         echo \"monitor-off: x=$x\"\n\
         shopt -u lastpipe\n\
         echo again | read x\n\
         echo \"lastpipe-off: x=$x\"",
    );
    assert_eq!(
        stdout,
        "monitor-on: x=before\nmonitor-off: x=world\nlastpipe-off: x=world\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
