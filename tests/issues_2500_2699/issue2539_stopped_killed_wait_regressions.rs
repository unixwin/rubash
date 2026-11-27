//! A stopped job that is then CONT+KILLed: `wait` answers from the
//! RECORDED state (rubash#2539; GNU bash 5.3.20 oracle-verified
//! 2026-11-02, cluster-F probe shape).
//!
//! GNU wait_for (jobs.c:3064) calls waitchld only while the child
//! is recorded RUNNING; for a job recorded STOPPED it returns
//! 128+WSTOPSIG (jobs.c:3255-3257) from the recorded state —
//! warning only for `%` operands (wait_for_job, jobs.c:3409-3410),
//! silent for a pid operand (wait_for_single_pid) — and the wait
//! itself never reaps the pending death, so it produces no
//! "Killed" notice. The death is only recorded at a LATER reap
//! point outside the wait (an intervening foreground wait turns
//! the same sequence into 137 + notice — the control pin below).
//! Rubash's wait-entry refresh published the deferred SIGCONT
//! mark and applied the death completion before the recorded
//! state was consulted, answering 137 and printing the notice.
//!
//! Not pinned: the listing state immediately after such a wait.
//! GNU's own SIGCHLD handler races it (verbatim drive: `Stopped`;
//! with a 0.5s pause before `jobs`: `Killed`) — an ENV-race, not a
//! contract.

#![cfg(unix)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn rubash_bin() -> String {
    env!("CARGO_BIN_EXE_rubash").to_string()
}

/// Run a script FILE under the built rubash; return (stdout, stderr).
fn run_script(tag: &str, body: &str) -> (String, String) {
    let dir = std::env::temp_dir().join(format!("issue2539-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    let path: PathBuf = dir.join("case.sh");
    fs::write(&path, body).expect("write script");
    let output = Command::new(rubash_bin())
        .arg(&path)
        .current_dir(&dir)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// Poll (bounded) until the shell's own `jobs -s` shows the job
/// Stopped, i.e. the stop has been RECORDED — the precondition of
/// every case in this file.
const POLL: &str =
    "for i in 1 2 3 4 5 6 7 8 9 10; do if jobs -s | grep -q Stopped; then break; fi; sleep 0.2; done\n";

#[test]
fn issue2539_stopped_cont_killed_wait_reports_stop_silently() {
    // The cluster-F shape: STOP recorded, then CONT+KILL, then the
    // wait, and the script ends — so any output beyond the verdict
    // could only have come from the wait itself. GNU: rc=147 and
    // total silence (no Killed notice from the wait or at exit).
    let (stdout, stderr) = run_script(
        "contkill",
        &format!(
            "set -m\n\
             sleep 30 & p=$!\n\
             kill -STOP $p\n\
             {POLL}\
             kill -CONT $p; kill -KILL $p; wait $p; echo \"wait rc=$?\"\n"
        ),
    );
    assert_eq!(stdout, "wait rc=147\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2539_wait_on_stopped_job_pid_form_is_silent_147() {
    let (stdout, stderr) = run_script(
        "stoppedpid",
        &format!(
            "set -m\n\
             sleep 30 & p=$!\n\
             kill -STOP $p\n\
             {POLL}\
             wait $p; echo rc=$?\n\
             kill -KILL $p; wait $p 2>/dev/null\n"
        ),
    );
    assert_eq!(stdout, "rc=147\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2539_wait_on_stopped_job_spec_form_warns_147() {
    // The `%` operand routes to wait_for_job (jobs.c:3409-3410):
    // same 147, plus the stopped warning.
    let (stdout, stderr) = run_script(
        "stoppedspec",
        &format!(
            "set -m\n\
             sleep 30 & p=$!\n\
             kill -STOP $p\n\
             {POLL}\
             wait %1; echo rc=$?\n\
             kill -KILL $p; wait $p 2>/dev/null\n"
        ),
    );
    assert_eq!(stdout, "rc=147\n");
    assert!(
        stderr.contains("warning: wait_for_job: job 1 is stopped"),
        "stderr should carry the wait_for_job warning, got: {stderr}"
    );
}

#[test]
fn issue2539_death_recorded_before_wait_still_reports_137() {
    // Control: when a real reap point (a foreground wait) records
    // the death BEFORE the operand wait runs, GNU answers with the
    // death itself — 137 and the Killed notice — because wait_for
    // now finds the job recorded DEAD, not STOPPED. The recorded-
    // state gate must not swallow genuinely recorded deaths.
    let (stdout, stderr) = run_script(
        "recorded",
        &format!(
            "set -m\n\
             sleep 30 & p=$!\n\
             kill -STOP $p\n\
             {POLL}\
             kill -CONT $p; kill -KILL $p\n\
             sleep 0.5\n\
             wait $p; echo rc=$?\n"
        ),
    );
    assert_eq!(stdout, "rc=137\n");
    assert!(
        stderr.contains("Killed"),
        "the recorded death's notice belongs to this wait, got stderr: {stderr}"
    );
}
