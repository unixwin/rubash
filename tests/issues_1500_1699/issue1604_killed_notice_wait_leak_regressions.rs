//! Issue rubash#1604 (residual of #1303): a `Killed` notice for a job
//! the shell's own `kill` just terminated must not be printed by the
//! boundary reap that ends the kill command itself — GNU learns the
//! death asynchronously (jobs.c sigchld_handler -> waitchld) and the
//! following `wait $p` claims the job, suppressing its notice
//! entirely (wait_for's notify_and_cleanup marks the waited job
//! notified). Rubash's synchronous reap caught the zombie on the
//! kill command's own boundary in ~1/6 of runs and flushed the notice
//! to raw stderr, straight past the wait's `2>/dev/null`.
//!
//! The settle form below is the stable discriminator measured
//! against GNU bash 5.3.20: stdout `wait=137` / `bare=0`, and stderr
//! carries exactly ONE `Killed` notice — the second job's, printed at
//! a later notification point where no redirection covers it.

use std::process::Command;

const SEED: &str = "\
sleep 50 &
p=$!
kill -KILL %1
wait $p 2>/dev/null; echo \"wait=$?\"
sleep 50 &
kill -KILL %1
sleep 1
wait 2>/dev/null; echo \"bare=$?\"
";

#[test]
fn killed_notice_does_not_leak_past_wait_redirect() {
    // The pre-fix failure was intermittent (~1/6), so the seed runs
    // several times; every single run must show exactly one notice.
    for run in 0..6 {
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg("-c")
            .arg(SEED)
            .output()
            .expect("run rubash");
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        assert!(
            stdout.contains("wait=137") && stdout.contains("bare=0"),
            "run {run}: wait statuses wrong: stdout={stdout:?} stderr={stderr:?}"
        );
        let notices = stderr.matches("Killed").count();
        assert_eq!(
            notices, 1,
            "run {run}: expected exactly 1 Killed notice (the second job's), got {notices}: stderr={stderr:?}"
        );
    }
}
