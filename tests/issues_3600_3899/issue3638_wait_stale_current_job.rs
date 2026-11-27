//! rubash#3638 — `wait %%` / `wait %+` (the current-job specs) after
//! the job table drains resolved a stale pointer: the first lookup
//! silently returned 0 (the newest dead-but-unreaped entry's saved
//! status) and only the second lookup reported "no such job".
//!
//! GNU's model (builtins/common.c:691 get_job_spec, jobs.h:115
//! INVALID_JOB): `%`, `%%`, `%+` return js.j_current VERBATIM — there
//! is no newest-job fallback — and the caller rejects NO_JOB or a
//! slot whose job is gone. When the last operand waits retire the
//! jobs, wait_for's tail notify_and_cleanup (jobs.c:3385 ->
//! cleanup_dead_jobs -> delete_job -> reset_current, jobs.c:3767)
//! clears j_current/j_previous to NO_JOB once no stopped or running
//! candidate remains, so `%%`/`%+` answer 127 "no such job" on the
//! very first lookup even while a notified corpse still occupies a
//! slot. `%-` reads js.j_previous the same way.
//!
//! Expectations cross-checked against GNU bash 5.3.20 (script-file
//! probes, 2026-11-18): `wait %%`/`wait %+`/`wait %` after
//! `wait %1; wait %2` all print `wait: <spec>: no such job` and
//! return 127; `wait %%` while the current job still runs returns 0;
//! `wait %%` on the same line right after `wait %1` still returns 0
//! because the current marker legitimately names the just-waited
//! corpse until the next cleanup pass.

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
    let dir = std::env::temp_dir().join(format!("issue3638_{}_{}", std::process::id(), seq));
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
fn wait_percent_percent_after_drain_is_127_with_diagnostic() {
    let run =
        run_script("set -m; sleep 0.1 & sleep 0.2 & wait %1; wait %2; wait %%; echo \"R=$?\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "R=127\n", "stdout: {}", run.stdout);
    assert!(
        run.stderr.contains("wait: %%: no such job"),
        "stderr: {}",
        run.stderr
    );
    let _ = run.dir;
}

#[test]
fn wait_percent_plus_after_drain_is_127_with_diagnostic() {
    let run =
        run_script("set -m; sleep 0.1 & sleep 0.2 & wait %1; wait %2; wait %+; echo \"R=$?\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "R=127\n", "stdout: {}", run.stdout);
    assert!(
        run.stderr.contains("wait: %+: no such job"),
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn wait_percent_percent_twice_after_drain_is_127_127() {
    // No one-shot stale hit: the first lookup must already fail, so
    // the second cannot succeed either.
    let run = run_script(
        "set -m; sleep 0.1 & sleep 0.2 & wait %1; wait %2; wait %% 2>/dev/null; echo \"R1=$?\"; wait %% 2>/dev/null; echo \"R2=$?\"\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "R1=127\nR2=127\n", "stdout: {}", run.stdout);
}

#[test]
fn wait_bare_percent_after_drain_is_127() {
    // `wait %` shares the js.j_current arm (common.c:724 case 0).
    let run = run_script(
        "set -m; sleep 0.1 & sleep 0.2 & wait %1; wait %2; wait % 2>/dev/null; echo \"R=$?\"\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "R=127\n", "stdout: {}", run.stdout);
}

#[test]
fn wait_percent_percent_with_no_jobs_ever_is_127() {
    // Control: the empty-table case already agreed before the fix.
    let run = run_script("set -m; wait %% 2>/dev/null; echo \"R=$?\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "R=127\n", "stdout: {}", run.stdout);
}

#[test]
fn wait_percent_minus_after_drain_is_127() {
    // Control: `%-` reads js.j_previous, which reset_current cleared
    // on the same pass — it never had the fallback.
    let run = run_script(
        "set -m; sleep 0.1 & sleep 0.2 & wait %1; wait %2; wait %- 2>/dev/null; echo \"R=$?\"\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "R=127\n", "stdout: {}", run.stdout);
}

#[test]
fn wait_percent_percent_while_current_job_runs_is_0() {
    // Control: a live current job still resolves and is waited on.
    let run = run_script("set -m; sleep 0.1 & sleep 0.2 & wait %%; echo \"R=$?\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "R=0\n", "stdout: {}", run.stdout);
}

#[test]
fn wait_percent_percent_same_line_after_waiting_current_is_0() {
    // The current marker legitimately names the just-waited corpse
    // until the next cleanup pass: GNU's wait_for_job returns the
    // recorded status (0) here, so the spec must still resolve.
    let run = run_script("set -m; sleep 0.05 & wait %1; wait %%; echo \"R=$?\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "R=0\n", "stdout: {}", run.stdout);
    assert!(
        !run.stderr.contains("no such job"),
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn explicit_pid_rewait_keeps_cached_status() {
    // Control: `wait %1` re-wait retains the saved status in both
    // shells (rubash#2777/#2896 territory, unaffected by #3638).
    let run = run_script("set -m; sleep 0.1 & wait %1; echo \"R1=$?\"; wait %1; echo \"R2=$?\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "R1=0\nR2=0\n", "stdout: {}", run.stdout);
}

#[test]
fn explicit_pid_rewait_keeps_nonzero_exit() {
    let run = run_script(
        "set -m; (sleep 0.1; exit 4) & wait %1; echo \"R1=$?\"; wait %1; echo \"R2=$?\"\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "R1=4\nR2=4\n", "stdout: {}", run.stdout);
}

#[test]
fn jobs_table_is_empty_after_both_waits() {
    // Control: the listing agrees the table drained; only the
    // current-job spec used to disagree.
    let run = run_script(
        "set -m; sleep 0.1 & sleep 0.2 & wait %1; wait %2; j=$(jobs); [ -z \"$j\" ] && echo \"R=empty\" || echo \"R=nonempty\"\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "R=empty\n", "stdout: {}", run.stdout);
}
