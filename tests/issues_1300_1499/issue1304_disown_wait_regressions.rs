//! rubash#1304 — `wait $pid` after `disown %1` errored with rc=127;
//! GNU silently returns 0 for the disowned (but still child) pid.
//!
//! Probe: rubash-fix2/k6-scratch/repro/1304.sh.
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
fn wait_for_disowned_pid_is_silent_zero() {
    let (stdout, _stderr, code) = rubash(
        "sleep 50 &\np1=$!\nsleep 51 &\np2=$!\ndisown %1\nwait $p1; echo \"wait_p1=$?\"\nkill $p1 $p2 2>/dev/null; wait 2>/dev/null; echo done",
    );
    assert_eq!(stdout, "wait_p1=0\ndone\n");
    assert_eq!(code, Some(0));
}
