//! rubash#1461 — killing $BASHPID inside a subshell killed the WHOLE
//! shell: BASHPID resolved to the main process pid even inside a
//! (forked) subshell. GNU: BASHPID is the current (sub)shell's pid,
//! so the subshell dies (rc=137, Killed notice) and the main shell
//! continues. ($$ staying the main pid is correct in both.)
//!
//! Probe: clusters/x1/issues/issue-1461.md repro.
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
fn killing_bashpid_in_subshell_spares_main_shell() {
    let (stdout, stderr, code) =
        rubash("( kill -KILL $BASHPID ); echo \"subshell_rc=$?\"\necho \"main_alive\"");
    assert_eq!(stdout, "subshell_rc=137\nmain_alive\n");
    assert!(stderr.contains("Killed"), "stderr: {stderr}");
    assert_eq!(code, Some(0));
}
