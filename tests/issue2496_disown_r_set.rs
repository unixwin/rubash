//! rubash#2496 — `disown -r` (and the `-h -r` marking arm) must
//! restrict the affected set to RUNNING jobs, the way GNU's
//! `delete_all_jobs (running_only)` / `nohup_all_jobs (running)` do.
//!
//! GNU derivation (bash 5.3 sources):
//! - `delete_all_jobs (running_only)` (jobs.c:5066): the sweep deletes a
//!   job only when `running_only == 0 || ISRUNNING (job)` (jobs.c:5091,
//!   `ISRUNNING` = the job's last process state is JRUNNING). Stopped and
//!   already-completed jobs stay in the table. Before sweeping it clears
//!   the current/previous markers (`js.j_current = js.j_previous =
//!   NO_JOB`, jobs.c:5078-5079), so a surviving job lists with NO `+`/`-`
//!   marker afterwards. Deletion goes through `delete_job` with
//!   `DEL_NOBGPID`, so the deleted job's processes stop being waitable
//!   children (`wait <pid>` -> 127 "not a child of this shell").
//! - `nohup_all_jobs (running_only)` (jobs.c:5111) applies the same
//!   JRUNNING filter when `disown -h -r` marks jobs J_NOHUP.
//! - The `-r` sweep never deletes a stopped job, so it never raises the
//!   DEL_WARNSTOPPED warning (jobs.c:1495-1496) for one; a stopped
//!   SURVIVOR keeps answering `wait %n` with the wait_for_job stopped
//!   warning + 147 (jobs.c:3409-3410 via builtins/wait.def:316) and
//!   `fg %n` normally.
//!
//! Before this fix the executor's All arm cleared / marked the WHOLE
//! table regardless of `running_only` (cluster E's REPORT §5; E had
//! already threaded the flag through the disown parser for #2489).
//!
//! Verification: every shape below was byte-compared against the GNU
//! bash 5.3.20 oracle binary on 2026-11-02 (identical stdout/stderr
//! modulo pid values; the `bash: line 1:` prefix is rubash's
//! established -c rendering, byte-identical to GNU invoked under the
//! name `bash`). One face is deliberately NOT byte-pinned: whether a
//! just-completed job still appears in a `jobs` listing as `Done` is
//! racy IN GNU ITSELF (both listings observed across identical runs —
//! the completed job is dropped once bash's notify pass reaches it,
//! timing-dependent in scripts). The completed-job pin therefore
//! asserts the deterministic subset (the stopped survivor, via a
//! `jobs | grep` read); the not-deleted guarantee for completed jobs is
//! the jobs.c:5091 state condition itself, oracle-confirmed in the
//! cluster probes where GNU listed `[1] Done` after `disown -r`.
//!
//! `disown -h -r`'s marking scope is likewise only half observable from
//! a script: J_NOHUP is consumed by `hangup_all_jobs`, which runs only
//! on the interactive-login SIGHUP / `huponexit` path (shell.c:1019,
//! sig.c:621-622). The `-h -r` pin locks the observable shape (nothing
//! is deleted, markers in listings are preserved, rc 0); the
//! running-only scope of the mark itself is code parity with
//! jobs.c:5111's filter.
//!
//! Job control needs `set -m` (in `-c` scripts it starts off). Each
//! test stops its job with SIGSTOP and polls `jobs -s` (which lists a
//! stopped job as `Done` once bash publishes the stop — the mirror of
//! `jobs` rendering `Stopped`) until the stop is visible before acting,
//! so the shapes do not depend on scheduling luck.

#![cfg(unix)]

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

const POLL: &str = "for i in 1 2 3 4 5 6 7 8 9 10; do if jobs -s | grep -q Stopped; then break; fi; sleep 0.2; done";

fn first_line_value(stdout: &str, prefix: &str) -> String {
    stdout
        .lines()
        .find_map(|line| line.strip_prefix(prefix))
        .unwrap_or_else(|| panic!("missing {prefix} line in stdout: {stdout:?}"))
        .trim()
        .to_string()
}

/// One running job (30s) + one stopped job (3s): `disown -r` deletes
/// only the running one; the stopped survivor lists with NO current /
/// previous marker (GNU clears both markers before the sweep) and
/// stays waitable (147 while stopped, 0 after CONT + completion).
#[test]
fn disown_r_deletes_only_running_jobs() {
    let script = format!(
        "set -m; sleep 30 & run=$!; sleep 3 & stp=$!; kill -STOP $stp; {POLL}; \
         echo before:; jobs; disown -r; echo \"disown rc=$?\"; echo after:; jobs; \
         wait %2; echo \"survivor-wait=$?\"; kill -CONT %2; wait %2; echo \"cleanup=$?\"; \
         kill -KILL $run 2>/dev/null"
    );
    let (stdout, stderr, code) = rubash(&script);
    assert_eq!(
        stdout,
        "before:\n\
         [1]-  Running                    sleep 30 &\n\
         [2]+  Stopped                    sleep 3\n\
         disown rc=0\n\
         after:\n\
         [2]   Stopped                    sleep 3\n\
         survivor-wait=147\n\
         cleanup=0\n"
    );
    assert_eq!(
        stderr,
        "bash: line 1: warning: wait_for_job: job 2 is stopped\n"
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
}

/// The stopped survivor of `disown -r` answers `wait %n` with the
/// stopped warning and 147, then completes normally after SIGCONT and
/// leaves the table.
#[test]
fn disown_r_stopped_survivor_still_waitable() {
    let script = format!(
        "set -m; sleep 2 & stp=$!; kill -STOP $stp; {POLL}; disown -r; \
         echo \"disown rc=$?\"; wait %1; echo \"wait-stopped rc=$?\"; \
         kill -CONT %1; wait %1; echo \"wait-after-cont rc=$?\"; jobs; echo \"jobs rc=$?\""
    );
    let (stdout, stderr, code) = rubash(&script);
    assert_eq!(
        stdout,
        "disown rc=0\nwait-stopped rc=147\nwait-after-cont rc=0\njobs rc=0\n"
    );
    assert_eq!(
        stderr,
        "bash: line 1: warning: wait_for_job: job 1 is stopped\n"
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
}

/// `fg` also still reaches the stopped survivor of `disown -r`: the
/// job's command line is echoed and it runs to completion in the
/// foreground.
#[test]
fn disown_r_stopped_survivor_fg() {
    let script = format!(
        "set -m; sleep 2 & stp=$!; kill -STOP $stp; {POLL}; disown -r; \
         fg %1; echo \"fg rc=$?\""
    );
    let (stdout, stderr, code) = rubash(&script);
    assert_eq!(stdout, "sleep 2\nfg rc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0), "stderr: {stderr}");
}

/// A running job deleted by `disown -r` is no longer a waitable child
/// (DEL_NOBGPID): `wait <pid>` reports 127 + the not-a-child diagnostic,
/// while the stopped survivor stays listed and waitable.
#[test]
fn disown_r_deleted_running_pid_not_a_child() {
    let script = format!(
        "set -m; sleep 30 & run=$!; echo \"run=$run\"; sleep 3 & stp=$!; \
         kill -STOP $stp; {POLL}; disown -r; echo \"disown rc=$?\"; \
         wait $run; echo \"wait-runpid rc=$?\"; jobs; \
         wait %2; echo \"survivor-wait=$?\"; kill -CONT %2; wait %2; echo \"cleanup=$?\"; \
         kill -KILL $run 2>/dev/null"
    );
    let (stdout, stderr, code) = rubash(&script);
    let run = first_line_value(&stdout, "run=");
    assert_eq!(
        stdout,
        format!(
            "run={run}\n\
             disown rc=0\n\
             wait-runpid rc=127\n\
             [2]   Stopped                    sleep 3\n\
             survivor-wait=147\n\
             cleanup=0\n"
        )
    );
    assert_eq!(
        stderr,
        format!(
            "bash: line 1: wait: pid {run} is not a child of this shell\n\
             bash: line 1: warning: wait_for_job: job 2 is stopped\n"
        )
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
}

/// A completed (Done, not yet notified) job is not JRUNNING either, so
/// `disown -r` must not delete it. GNU's own `Done` listing line is
/// racy (see the module docs), so this pin asserts the deterministic
/// subset: rc 0 and the stopped co-tenant still listed afterwards.
#[test]
fn disown_r_completed_job_not_deleted() {
    let script = format!(
        "set -m; sleep 0 & sleep 3 & stp=$!; kill -STOP $stp; {POLL}; \
         disown -r; echo \"disown rc=$?\"; jobs | grep Stopped; echo \"grep rc=$?\"; \
         wait %2; echo \"survivor-wait=$?\"; kill -CONT %2; wait %2; echo \"cleanup=$?\""
    );
    let (stdout, stderr, code) = rubash(&script);
    assert_eq!(
        stdout,
        "disown rc=0\n\
         [2]   Stopped                    sleep 3\n\
         grep rc=0\n\
         survivor-wait=147\n\
         cleanup=0\n"
    );
    assert_eq!(
        stderr,
        "bash: line 1: warning: wait_for_job: job 2 is stopped\n"
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
}

/// `disown -h -r` marks instead of deleting: the listing afterwards is
/// byte-identical to the one before (both jobs, markers intact) and rc
/// is 0. The running-only scope of the mark is not script-observable
/// (J_NOHUP is consumed only on the interactive hangup path) — see the
/// module docs; the fix applies jobs.c:5111's filter in code.
#[test]
fn disown_h_r_keeps_all_jobs_listed() {
    let script = format!(
        "set -m; sleep 30 & run=$!; sleep 3 & stp=$!; kill -STOP $stp; {POLL}; \
         echo before:; jobs; disown -h -r; echo \"disown rc=$?\"; echo after:; jobs; \
         wait %2; echo \"survivor-wait=$?\"; kill -CONT %2; wait %2; echo \"cleanup=$?\"; \
         kill -KILL $run 2>/dev/null"
    );
    let (stdout, stderr, code) = rubash(&script);
    assert_eq!(
        stdout,
        "before:\n\
         [1]-  Running                    sleep 30 &\n\
         [2]+  Stopped                    sleep 3\n\
         disown rc=0\n\
         after:\n\
         [1]-  Running                    sleep 30 &\n\
         [2]+  Stopped                    sleep 3\n\
         survivor-wait=147\n\
         cleanup=0\n"
    );
    assert_eq!(
        stderr,
        "bash: line 1: warning: wait_for_job: job 2 is stopped\n"
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
}

/// With only a running job present, `disown -r` empties the table: a
/// following bare `disown` finds no current job and `wait <pid>` is
/// 127.
#[test]
fn disown_r_sole_running_job_empties_table() {
    let script = "set -m; sleep 30 & run=$!; echo \"run=$run\"; disown -r; \
                  echo \"disown rc=$?\"; jobs; echo \"jobs rc=$?\"; disown; \
                  echo \"bare rc=$?\"; wait $run; echo \"wait rc=$?\"; kill -KILL $run";
    let (stdout, stderr, code) = rubash(script);
    let run = first_line_value(&stdout, "run=");
    assert_eq!(
        stdout,
        format!("run={run}\ndisown rc=0\njobs rc=0\nbare rc=1\nwait rc=127\n")
    );
    assert_eq!(
        stderr,
        format!(
            "bash: line 1: disown: current: no such job\n\
             bash: line 1: wait: pid {run} is not a child of this shell\n"
        )
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
}

/// Guard for the untouched arm: plain `disown -a` still clears the
/// WHOLE table (running and stopped), warning about the stopped job
/// (the #2489 face) exactly once.
#[test]
fn disown_a_still_clears_whole_table() {
    let script = format!(
        "set -m; sleep 30 & run=$!; sleep 3 & stp=$!; echo \"stp=$stp\"; \
         kill -STOP $stp; {POLL}; disown -a; echo \"disown rc=$?\"; \
         jobs; echo \"jobs rc=$?\"; kill -KILL $run $stp 2>/dev/null"
    );
    let (stdout, stderr, code) = rubash(&script);
    let stp = first_line_value(&stdout, "stp=");
    assert_eq!(stdout, format!("stp={stp}\ndisown rc=0\njobs rc=0\n"));
    assert_eq!(
        stderr,
        format!("bash: line 1: warning: deleting stopped job 2 with process group {stp}\n")
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
}
