//! rubash#2698 — a killed background job was listed by `jobs` AGAIN
//! after its death notice had already printed at a command boundary,
//! and a following `wait %1` still returned the death status instead
//! of failing.
//!
//! GNU's boundary step is notify_and_cleanup (jobs.c): printing the
//! notice marks the job J_NOTIFIED, and cleanup deletes a dead
//! notified job in the same pass. The notice path here printed
//! without marking (so cleanup_dead_jobs never retired the entry and
//! the `jobs` builtin printed the death a second time, marking it
//! one boundary too late), and even once marked, the boundary never
//! retired the entry (so the jobspec stayed resolvable). The fix
//! makes format_signal_notices report which pids' notices actually
//! printed, marks exactly those jobs notified at both print sites
//! (the boundary report and the wait drain), and runs
//! cleanup_dead_jobs at the boundary. Statuses survive in
//! completed_statuses (bgpids parity), so `wait <pid>` keeps working.
//!
//! Expectations cross-checked against GNU bash 5.3.20 (script-file
//! probes). The notice's pid and line stamp are normalized out of
//! the assertions: pids are inherently run-specific, and the stamp
//! carries a pre-existing one-line delta (the notice is stamped with
//! the just-finished command's line instead of the upcoming one),
//! which is outside this issue's scope.

use std::process::Command;

struct Run {
    status: i32,
    stdout: String,
    stderr: String,
}

fn run_script(body: &str) -> Run {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("issue2698_{}_{}", std::process::id(), seq));
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
    }
}

#[test]
fn jobs_does_not_relist_a_job_whose_death_notice_printed() {
    let run = run_script("sleep 5 &\nkill -KILL %1\nsleep 0.3\njobs\necho done\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "done\n", "stdout: {}", run.stdout);
    assert_eq!(
        run.stderr.matches("Killed").count(),
        1,
        "exactly one death notice; stderr: {}",
        run.stderr
    );
}

#[test]
fn wait_jobspec_after_boundary_notice_reports_no_such_job() {
    let run = run_script(
        "sleep 5 &\nkill -KILL %1\nsleep 0.3\nwait %1\necho \"waitrc=$?\"\njobs\necho done\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("waitrc=127"), "stdout: {}", run.stdout);
    assert!(run.stderr.contains("no such job"), "stderr: {}", run.stderr);
    assert_eq!(
        run.stderr.matches("Killed").count(),
        1,
        "the death notice prints once, at the boundary; stderr: {}",
        run.stderr
    );
}

#[test]
fn wait_jobspec_without_prior_notice_still_reaps_the_death() {
    // No intervening command boundary notices the death first: the
    // wait itself reaps it and returns the signal status (128+9).
    let run =
        run_script("sleep 5 &\nkill -KILL %1\nwait %1\necho \"waitrc=$?\"\njobs\necho done\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("waitrc=137"), "stdout: {}", run.stdout);
    assert_eq!(
        run.stdout.matches("Killed").count(),
        0,
        "stdout has no listing"
    );
}

#[test]
fn wait_by_pid_still_finds_the_status_after_retirement() {
    // bgpids parity: retiring the job from the table must not drop
    // its recorded status for an explicit-pid wait.
    let run = run_script(
        "sleep 5 &\npid=$!\nkill -KILL %1\nsleep 0.3\nwait $pid\necho \"waitrc=$?\"\necho done\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("waitrc=137"), "stdout: {}", run.stdout);
}

#[test]
fn normally_exited_job_is_not_a_signal_notice() {
    // Control: a job that exits normally produces no Killed notice.
    // (GNU parity, probe-verified: `jobs` lists the completed job
    // once with its Done status on stdout — only the signal-death
    // double-report was the defect.)
    let run = run_script("sleep 0.1 &\nsleep 0.3\njobs\necho done\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("Done"), "stdout: {}", run.stdout);
    assert!(run.stdout.contains("sleep 0.1"), "stdout: {}", run.stdout);
    assert!(run.stdout.ends_with("done\n"), "stdout: {}", run.stdout);
    assert!(!run.stderr.contains("Killed"), "stderr: {}", run.stderr);
}
