//! Issue rubash#3605 (class H, run-trap): an in-process `${THIS_SH} script`
//! child's end-of-script run_exit_trap latched the shared executor's
//! `exit_trap_ran`, so an EXIT trap the PARENT armed afterwards was silently
//! dropped (trap.tests tail: `trap 'echo exiting' 0; exit` printed nothing).
//!
//! GNU ownership: a `bash script` child is a separate process
//! (execute_cmd.c:6139 execute_disk_command forks and execs); run_exit_trap
//! (trap.c:1027) keys on the child's own sigmodes[EXIT_TRAP] — SIG_TRAPPED /
//! SIG_INPROGRESS are per-process and never leak into the parent.
//! execute_direct_shell_script (external_finish.rs) emulates that boundary
//! in-process and now parks/restores the parent's latch for the child run.
#![cfg(unix)]

use std::process::Command;

fn run(script: &str) -> (String, String) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue3605-trap-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("mkdir fixture");
    // trap6.sub's tail shape: the child arms an EXIT trap whose action
    // produces no output; its run at child end-of-script is what used to
    // leak the parent's exit_trap_ran latch.
    std::fs::write(
        dir.join("child.sub"),
        "ReturnFalse() { false; return; }\ntrap 'ReturnFalse && echo woops' EXIT\n",
    )
    .expect("write child");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .env_remove("BASH_ENV")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// GNU: the child's EXIT trap runs inside the child only; the parent's
/// re-armed EXIT trap still fires at the parent's real exit.
#[test]
fn issue3605_this_sh_child_exit_trap_does_not_latch_parent() {
    let script = r#"
${THIS_SH} ./child.sub
trap 'echo exiting' 0
exit
"#;
    let (stdout, stderr) = run(script);
    assert_eq!(stdout, "exiting\n", "stderr: {stderr}");
}

/// Same latch, second shape: several in-process children each fire their
/// own EXIT trap (visible output), then the parent's still fires.
#[test]
fn issue3605_parent_exit_trap_after_multiple_children() {
    let script = r#"
${THIS_SH} ./child.sub
${THIS_SH} ./child.sub
trap 'echo exiting' 0
echo before-exit
exit
"#;
    let (stdout, stderr) = run(script);
    assert_eq!(stdout, "before-exit\nexiting\n", "stderr: {stderr}");
}
