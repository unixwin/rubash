//! rubash#2896 — `wait` re-wait cadence diverged from GNU in both
//! directions. GNU's model (jobs.c): wait_for_job marks the job
//! J_NOTIFIED after wait_for returns (jobs.c:3434-3437); the entry
//! is deleted inside the NEXT wait's tail cleanup (jobs.c:3385 ->
//! cleanup_dead_jobs, jobs.c:1303), after resolution, with the
//! status saved into bgpids — so within one command list `wait %1`
//! succeeds twice and fails the third time, while a wait on a
//! later line finds the entry already retired by the boundary
//! notification pass (notify_and_cleanup runs per read list,
//! eval.c:355) and fails at once for a jobspec (a pid wait still
//! succeeds via bgpids). A bare wait additionally runs bgp_clear
//! (jobs.c:2863-2866), wiping the saved statuses wholesale — even
//! when the job table is already empty.
//!
//! Two rubash defects broke this: the boundary notice reporter ran
//! cleanup_dead_jobs unconditionally at every command boundary
//! (retiring the entry one wait too early, mid-list), and the bare
//! wait never cleared the saved-status list.
//!
//! Expectations cross-checked against GNU bash 5.3.20 (script-file
//! probes s96a-s96m in the r75 hunt). The issue text's claim that
//! `wait $p` twice on separate lines gives 127 in both shells did
//! not reproduce in any probed shape: pid waits keep succeeding
//! via bgpids; only jobspec waits and post-bare-wait waits fail.

use std::path::PathBuf;
use std::process::Command;

struct Run {
    status: i32,
    stdout: String,
    stderr: String,
    dir: PathBuf,
}

fn run_script(body: &str) -> Run {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("issue2896_{}_{}", std::process::id(), seq));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    std::fs::write(dir.join("case.sh"), body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    Run {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        dir,
    }
}

#[test]
fn jobspec_rewait_within_one_list_is_zero_zero_127() {
    let run = run_script(
        "sleep 0.05 & wait %1; echo \"w1:$?\"; wait %1; echo \"w2:$?\"; wait %1; echo \"w3:$?\"\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "w1:0\nw2:0\nw3:127\n", "stdout: {}", run.stdout);
    assert!(run.stderr.contains("no such job"), "stderr: {}", run.stderr);
    let _ = run.dir;
}

#[test]
fn jobspec_rewait_on_a_later_line_is_127() {
    let run = run_script("sleep 0.05 &\nwait %1; echo \"w1:$?\"\nwait %1; echo \"w2:$?\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "w1:0\nw2:127\n", "stdout: {}", run.stdout);
}

#[test]
fn pid_rewait_keeps_succeeding_via_saved_status() {
    let run = run_script(
        "sleep 0.05 & p=$!\nwait $p; echo \"w1:$?\"\nwait $p; echo \"w2:$?\"\nwait $p; echo \"w3:$?\"\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "w1:0\nw2:0\nw3:0\n", "stdout: {}", run.stdout);
}

#[test]
fn killed_job_rewait_within_one_list_is_143_143_127() {
    // Same two-successes-then-127 cadence as the plain-exit case;
    // the successes return the signal death status (143).
    let run = run_script(
        "sleep 0.2 & kill %1; wait %1; echo \"w1:$?\"; wait %1; echo \"w2:$?\"; wait %1; echo \"w3:$?\"\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout, "w1:143\nw2:143\nw3:127\n",
        "stdout: {}",
        run.stdout
    );
}

#[test]
fn killed_job_rewait_on_a_later_line_is_143_then_127() {
    let run =
        run_script("sleep 0.2 &\nkill %1\nwait %1; echo \"w1:$?\"\nwait %1; echo \"w2:$?\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "w1:143\nw2:127\n", "stdout: {}", run.stdout);
}

#[test]
fn bare_wait_wipes_saved_statuses() {
    let run = run_script(
        "sleep 0.05 & p=$!\nwait; echo \"wall:$?\"\nwait $p; echo \"rc:$?\"\nwait %1; echo \"spec:$?\"\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout, "wall:0\nrc:127\nspec:127\n",
        "stdout: {}",
        run.stdout
    );
}

#[test]
fn bare_wait_after_an_explicit_wait_still_wipes_saved_statuses() {
    // The job entry is already retired when the bare wait runs (the
    // preceding line boundary cleaned it up), so the bare wait's
    // bgp_clear must run on the no-jobs exit too.
    let run = run_script(
        "sleep 0.05 & p=$!\nwait $p; echo \"w1:$?\"\nwait; echo \"wall:$?\"\nwait $p; echo \"w2:$?\"\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout, "w1:0\nwall:0\nw2:127\n",
        "stdout: {}",
        run.stdout
    );
}
