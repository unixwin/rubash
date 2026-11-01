//! Signaling a dead-but-unwaited job through its jobspec (rubash#2381;
//! GNU bash 5.3.20 oracle-verified 2026-10-31).
//!
//! GNU's contract is split by operand FORM. The pid arm is a plain
//! kill(2) and takes its verdict (builtins/kill.def:213 kill_pid ->
//! jobs.c:4023): once waitchld has reaped the child, that is ESRCH.
//! The jobspec arm instead walks the retained job's pipeline
//! (kill.def:262 -> jobs.c:3988-4007 kill_pid): live members are
//! signaled in order and the walk stops at the first member whose
//! death is already recorded (jobs.c:3999-4000's `PALIVE (p) == 0`
//! skip never advances the cursor — the delivery set is the live
//! prefix), and the per-member results never fail the operand — so
//! while the job entry is retained, signaling it succeeds vacuously
//! when nothing live remains, for real signals and for the -0 probe
//! alike.
//! After `wait` consumes the entry, the jobspec is "no such job" and
//! the pid is ESRCH again. Rubash collapsed the jobspec to the job's
//! last pid and judged that pid by raw kill(2) / the completed-status
//! table, printing `(pid) - No such process` for the retained dead
//! job; these pins assert the oracle's settled-state behavior (the
//! death is observed across a foreground `sleep 1`, the same
//! observation point at which GNU's waitchld reaps — the fresh-death
//! window between death and reap is racy in GNU itself and is not
//! pinned).

#![cfg(unix)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn rubash_bin() -> String {
    env!("CARGO_BIN_EXE_rubash").to_string()
}

/// Run a script FILE under the built rubash; return (stdout, stderr).
fn run_script(tag: &str, body: &str) -> (String, String) {
    let dir = std::env::temp_dir().join(format!("issue2381-{tag}-{}", std::process::id()));
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

#[test]
fn issue2381_jobspec_signals_on_retained_dead_job_succeed() {
    let (stdout, stderr) = run_script(
        "spec-settled",
        "sleep 0.2 & p=$!\n\
         sleep 1\n\
         kill -TERM %1; echo \"spec TERM rc=$?\"\n\
         kill -TERM %1; echo \"spec TERM again rc=$?\"\n\
         kill -KILL %1; echo \"spec KILL rc=$?\"\n\
         kill -0 %1; echo \"spec 0 rc=$?\"\n",
    );
    assert_eq!(
        stdout,
        "spec TERM rc=0\n\
         spec TERM again rc=0\n\
         spec KILL rc=0\n\
         spec 0 rc=0\n"
    );
    // The vacuous walk success is silent: no ESRCH diagnostic at all.
    assert_eq!(stderr, "");
}

#[test]
fn issue2381_jobspec_fresh_exit_job_succeeds() {
    // The job exits on its own (status 3) before any signal: a
    // handshake file plus a short foreground sleep make the death —
    // and its recording — precede the kills deterministically (an
    // unsynchronized `(exit 3) & kill ...` races the spawn: the
    // TERM can land while the subshell is still alive and the wait
    // status becomes 143 instead of 3, in GNU as well).
    let (stdout, stderr) = run_script(
        "spec-fresh-exit",
        "( echo x > ready.txt; exit 3 ) & p=$!\n\
         while [ ! -f ready.txt ]; do sleep 0.05; done\n\
         sleep 0.3\n\
         kill -TERM %1; echo \"spec TERM rc=$?\"\n\
         kill -0 %1; echo \"spec 0 rc=$?\"\n\
         wait $p; echo \"wait rc=$?\"\n",
    );
    assert_eq!(
        stdout,
        "spec TERM rc=0\n\
         spec 0 rc=0\n\
         wait rc=3\n"
    );
    assert_eq!(stderr, "");
}

#[test]
fn issue2381_pid_form_and_after_wait_controls_stay_esrch() {
    // The pid arm is kill(2)'s verdict only (jobs.c:4023): a child the
    // shell has reaped reports ESRCH even while its job is retained,
    // and everything reports ESRCH / no such job once `wait` consumed
    // the entry. GNU 5.3.20 agrees on every line of this script.
    let (stdout, stderr) = run_script(
        "pid-controls",
        "sleep 0.2 & p=$!\n\
         echo \"child=$p\"\n\
         sleep 1\n\
         kill -TERM $p; echo \"pid TERM rc=$?\"\n\
         kill -0 $p; echo \"pid 0 rc=$?\"\n\
         wait $p; echo \"wait rc=$?\"\n\
         kill -TERM $p; echo \"pid TERM after-wait rc=$?\"\n\
         kill -TERM %1; echo \"spec TERM after-wait rc=$?\"\n\
         kill -TERM 999999; echo \"pid TERM never rc=$?\"\n",
    );
    let pid = stdout
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("child="))
        .expect("script echoes its child pid")
        .to_string();
    assert_eq!(
        stdout,
        format!(
            "child={pid}\n\
             pid TERM rc=1\n\
             pid 0 rc=1\n\
             wait rc=0\n\
             pid TERM after-wait rc=1\n\
             spec TERM after-wait rc=1\n\
             pid TERM never rc=1\n"
        )
    );
    // pid TERM, pid 0 and pid TERM after-wait each print the ESRCH
    // diagnostic for the child pid; the after-wait spec prints
    // "no such job"; the never-known pid prints its own ESRCH.
    assert_eq!(
        stderr
            .matches(&format!("({pid}) - No such process"))
            .count(),
        3,
        "stderr: {stderr}"
    );
    assert!(stderr.contains("kill: %1: no such job"), "stderr: {stderr}");
    assert!(
        stderr.contains("(999999) - No such process"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue2381_jobspec_delivery_to_live_job_still_kills() {
    // The walk must not blunt real delivery: a live job signaled
    // through its spec dies by that signal (wait reports 128+TERM).
    let (stdout, stderr) = run_script(
        "spec-live",
        "sleep 60 & p=$!\n\
         kill -TERM %1; echo \"kill rc=$?\"\n\
         wait $p; echo \"wait rc=$?\"\n",
    );
    assert_eq!(stdout, "kill rc=0\nwait rc=143\n");
    assert!(!stderr.contains("No such process"), "stderr: {stderr}");
}

#[test]
fn issue2381_jobspec_pipeline_signals_every_live_member() {
    // kill_pid delivers to the whole pipeline (jobs.c:3988-4007):
    // with all members live, a spec TERM ends every stage — the job
    // leaves the table (oracle-verified: wait rc=143, jobs empty).
    // Rubash reaches the same end state through the pipeline's
    // supervisor member.
    let (stdout, stderr) = run_script(
        "spec-pipe-live",
        "sleep 60 | sleep 60 & p=$!\n\
         kill -TERM %1; echo \"kill rc=$?\"\n\
         wait $p; echo \"wait rc=$?\"\n\
         n=0; left=1\n\
         while [ $n -lt 40 ]; do\n\
           jobs > jobs.txt\n\
           [ -s jobs.txt ] || { left=0; break; }\n\
           sleep 0.05; n=$((n+1))\n\
         done\n\
         echo \"jobsleft=$left\"\n",
    );
    assert_eq!(stdout, "kill rc=0\nwait rc=143\njobsleft=0\n");
    assert!(!stderr.contains("No such process"), "stderr: {stderr}");
}

#[test]
fn issue2381_jobspec_mixed_pipeline_dead_stage_does_not_esrch() {
    // Live head + already-dead tail stage: the spec operand must not
    // report ESRCH for the dead stage — the job is signaled through
    // its live part and completes (oracle basis: kill_pid's walk,
    // jobs.c:3988-4007, plus strace showing delivery proceeding past
    // the dead member's slot without an ESRCH verdict; GNU itself
    // spins on this shape once the death is recorded, so the pin
    // asserts the delivery outcome and the success status rubash
    // reports, which match every case in which GNU returns).
    // Rubash realizes a background pipeline behind one supervisor
    // member in the job table, so the delivery lands on that member
    // and the whole pipeline ends — the same end state GNU reaches.
    let (stdout, stderr) = run_script(
        "spec-pipe-dead-tail",
        "sleep 60 | (exit 3) & p=$!\n\
         sleep 1\n\
         kill -TERM %1; echo \"kill rc=$?\"\n\
         n=0; left=1\n\
         while [ $n -lt 40 ]; do\n\
           jobs > jobs.txt\n\
           [ -s jobs.txt ] || { left=0; break; }\n\
           sleep 0.05; n=$((n+1))\n\
         done\n\
         echo \"jobsleft=$left\"\n",
    );
    assert_eq!(stdout, "kill rc=0\njobsleft=0\n");
    assert!(!stderr.contains("No such process"), "stderr: {stderr}");
}

#[test]
fn issue2381_jobspec_posix_tally_treats_retained_dead_job_as_success() {
    // kill.def:283-286: posix mode fails the command only when a real
    // kill_pid attempt failed (any_failed). The vacuous walk success
    // is not a failure, so posix `kill -TERM %1` on the retained dead
    // job is rc 0 and silent; a following never-known pid still fails
    // the combined invocation.
    let (stdout, stderr) = run_script(
        "spec-posix",
        "set -o posix\n\
         sleep 0.2 & p=$!\n\
         sleep 1\n\
         kill -TERM %1 2>err.txt; echo \"posix spec TERM rc=$? errlines=$(wc -l < err.txt)\"\n\
         kill -TERM %1 999999; echo \"posix spec+never rc=$?\"\n",
    );
    assert_eq!(
        stdout,
        "posix spec TERM rc=0 errlines=0\n\
         posix spec+never rc=1\n"
    );
    assert!(
        stderr.contains("(999999) - No such process"),
        "stderr: {stderr}"
    );
    assert_eq!(
        stderr.matches("No such process").count(),
        1,
        "stderr: {stderr}"
    );
}
