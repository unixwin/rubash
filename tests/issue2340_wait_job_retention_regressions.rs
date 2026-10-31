//! rubash#2340 — terminated-job entry retention after `kill` + `wait`.
//!
//! GNU's jobs-table lifetime law (bash-5.3 jobs.c):
//! - A wait never deletes the entry it waited for: wait_for_job
//!   (jobs.c:3434-3437) and wait_for_single_pid (jobs.c:2776-2779) only
//!   mark the job J_NOTIFIED when the wait completes; the entry leaves
//!   at the NEXT cleanup_dead_jobs pass (jobs.c:1303), which runs inside
//!   a later wait's notify_and_cleanup (jobs.c:3385), before a `jobs`
//!   listing, or when a new job is created.
//! - Bare `wait` ends with mark_dead_jobs_as_notified(1) +
//!   cleanup_dead_jobs (wait_for_background_pids, jobs.c:2863-2864);
//!   the force pass (jobs.c:5192-5201) exempts the job whose last pid
//!   is last_asynchronous_pid ($!) in a non-interactive shell, so that
//!   one dead entry survives the wait and stays listed.
//! - Printing a job is its notification (pretty_print_job,
//!   jobs.c:2215-2219): a main-shell `jobs` shows the survivor once;
//!   the next listing's leading cleanup removes it.
//!
//! Expectations verified against GNU bash 5.3.20 (script files,
//! 2026-10-31). GNU itself races 0/1 on the kill-then-wait seed when a
//! death is processed during the wait instead of before it; the pins
//! encode GNU's settled/rule outcome, which is also the hunt verdict's
//! byte-stable result for the seed.

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2340-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run(tag: &str, script: &str) -> (String, String) {
    let dir = tempdir(tag);
    std::fs::write(dir.join("probe.sh"), script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let _ = std::fs::remove_dir_all(&dir);
    (stdout, stderr)
}

/// The issue seed: bare `wait` after killing both jobs keeps exactly
/// the $! job's entry (GNU: `done` then count 1; rubash deleted both).
#[test]
fn bare_wait_keeps_last_async_job_entry() {
    let (stdout, _) = run(
        "seed",
        "sleep 30 & sleep 30 & kill %1 %2; wait 2>/dev/null; echo done; jobs | wc -l\n",
    );
    assert_eq!(stdout, "done\n1\n");
}

/// The survivor is job %2 — the most recently started async job — and
/// it lists as Terminated.
#[test]
fn bare_wait_survivor_is_the_last_job() {
    let (stdout, _) = run(
        "survivor",
        "sleep 30 & sleep 30 & kill %1 %2; wait 2>/dev/null; jobs\n",
    );
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "stdout: {stdout}");
    assert!(lines[0].starts_with("[2]"), "stdout: {stdout}");
    assert!(lines[0].contains("Terminated"), "stdout: {stdout}");
    assert!(lines[0].contains("sleep 30"), "stdout: {stdout}");
}

/// Operand waits mark each waited job; the second wait's cleanup
/// collects the first, and the listing's leading cleanup collects the
/// second before printing — count 0 (GNU-verified).
#[test]
fn operand_waits_leave_no_entries() {
    let (stdout, _) = run(
        "operands",
        "sleep 30 & sleep 30 & kill %1 %2; wait %1 %2 2>/dev/null; echo done; jobs | wc -l\n",
    );
    assert_eq!(stdout, "done\n0\n");
}

/// Waiting for %1 only: %1 is collected by the listing's cleanup, the
/// never-waited %2 stays listed (GNU-verified).
#[test]
fn single_operand_wait_keeps_unwaited_job() {
    let (stdout, _) = run(
        "one-operand",
        "sleep 30 & sleep 30 & kill %1 %2; wait %1 2>/dev/null; echo done; jobs | wc -l; jobs\n",
    );
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3, "stdout: {stdout}");
    assert_eq!(lines[0], "done");
    assert_eq!(lines[1], "1");
    assert!(lines[2].starts_with("[2]"), "stdout: {stdout}");
    assert!(lines[2].contains("Terminated"), "stdout: {stdout}");
}

/// Jobs that already died before the wait (settled during a foreground
/// command) take the same endgame: only the $! job survives.
#[test]
fn settled_dead_jobs_keep_last_async_entry() {
    let (stdout, _) = run(
        "settled",
        "true & true & sleep 0.3; wait; echo done; jobs | wc -l\n",
    );
    assert_eq!(stdout, "done\n1\n");
}

/// A job the bare wait itself reaps live is consumed: no survivor.
#[test]
fn live_reaped_single_job_leaves_nothing() {
    let (stdout, _) = run("live", "sleep 0.2 & wait; echo done; jobs | wc -l\n");
    assert_eq!(stdout, "done\n0\n");
}

/// A second bare wait does not collect the survivor: it is still the
/// $! job and still unnotified (GNU-verified).
#[test]
fn repeated_bare_wait_keeps_survivor() {
    let (stdout, _) = run(
        "twice",
        "sleep 30 & sleep 30 & kill %1 %2; wait 2>/dev/null; jobs | wc -l; wait 2>/dev/null; jobs | wc -l\n",
    );
    assert_eq!(stdout, "1\n1\n");
}

/// In the main shell (no pipeline subshell), the first `jobs` prints
/// the survivor and marks it; the second listing's leading cleanup
/// removes it before printing (GNU-verified).
#[test]
fn main_shell_jobs_prints_survivor_once() {
    let (stdout, _) = run(
        "mainjobs",
        "sleep 30 & sleep 30 & kill %1 %2; wait 2>/dev/null\njobs > j1.txt; jobs > j2.txt\nwc -l < j1.txt; wc -l < j2.txt\n",
    );
    assert_eq!(stdout, "1\n0\n");
}

/// Retention does not lose statuses: repeated operand waits keep
/// reporting the killed jobs' 128+SIGTERM (GNU-verified).
#[test]
fn repeated_operand_wait_reports_status() {
    let (stdout, _) = run(
        "statuses",
        "sleep 30 & sleep 30 & kill %1 %2; wait %1; echo rc1=$?; wait %1; echo rc2=$?; wait %2; echo rc3=$?\n",
    );
    assert_eq!(stdout, "rc1=143\nrc2=143\nrc3=143\n");
}
