//! rubash#2241: bats' CTRL-C family — monitor-mode process grouping
//! and group-delivered SIGINT during an emulated foreground sleep.
//!
//! Face 1 (grouping): GNU groups every job under job control —
//! jobs.c:2390 make_child runs setpgid(mypid, pipeline_pgrp) in the
//! child and jobs.c:2455 repeats setpgid from the parent after fork,
//! so a single-process job's pgid IS its pid and `kill -INT -- -$!`
//! (the delivery bats-exec-file uses) reaches it. rubash recorded the
//! group in the job table (jobs -x) but never created it: the builtin
//! group kill answered ESRCH and the job survived.
//!
//! Face 2 (delivery): bats delivers SIGINT by kill(2) to the whole
//! process group. GNU's real `sleep` dies from that delivery, so
//! jobs.c:3064 wait_for returns at once and the pending INT trap
//! dispatches (trap.c:545). rubash's in-process sleep emulation kept
//! the rubash#1034 deferral for every kill(2) delivery, so the trap
//! sat queued for the sleep's full remainder (10s in the bats
//! fixture) and the outer driver's 1s liveness check killed the run.
//! The emulation now ends the virtual sleep when the delivery's
//! sender sits outside the shell's process group (a group-addressed
//! kill) while an in-group sender — one of the shell's own jobs
//! running `kill -INT $$`, the #1034 shape — still defers.
//!
//! All expectations verified against bash 5.3.20 via script files.

#![cfg(unix)]

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn run_rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Poll a job's pgid (bounded, ~2s) and echo both ids. Shared by the
/// grouping pins: parent-side setpgid makes the group exist at spawn
/// return, the poll only absorbs ps startup jitter.
const PGID_POLL: &str = "\
pgid=\n\
for i in $(seq 1 40); do\n\
  pgid=$(ps -o pgid= -p \"$pid\" 2>/dev/null)\n\
  pgid=${pgid//[[:space:]]/}\n\
  [ -n \"$pgid\" ] && break\n\
  sleep 0.05\n\
done\n\
echo \"pid=$pid pgid=$pgid\"";

/// Monitor mode: a background external command leads its own process
/// group, a builtin group kill reaches it, and `wait` reports the
/// signal death (GNU jobs.c:2390/2455). Baseline rubash: pgid is the
/// inherited harness group, the group kill prints ESRCH, the job
/// survives.
#[test]
fn monitor_background_job_leads_own_group() {
    let (stdout, stderr, code) = run_rubash(&format!(
        "set -m\n\
         sleep 30 &\n\
         pid=$!\n\
         {PGID_POLL}\n\
         [ \"$pgid\" = \"$pid\" ] && echo GROUPED || echo SHARED\n\
         kill -TERM -- -$pid 2>/dev/null && echo GROUPKILL-OK || echo GROUPKILL-ESRCH\n\
         for i in $(seq 1 40); do kill -0 \"$pid\" 2>/dev/null || break; sleep 0.05; done\n\
         kill -0 \"$pid\" 2>/dev/null && echo STILL-ALIVE || echo GONE\n\
         kill -9 \"$pid\" 2>/dev/null\n\
         wait \"$pid\" 2>/dev/null; echo \"wait rc=$?\""
    ));
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains("GROUPED"),
        "job must lead its group:\n{stdout}"
    );
    assert!(
        stdout.contains("GROUPKILL-OK"),
        "builtin group kill must find the group:\n{stdout}"
    );
    assert!(
        stdout.contains("GONE"),
        "job must die from the group TERM:\n{stdout}"
    );
    assert!(
        stdout.contains("wait rc=143"),
        "wait reports 128+SIGTERM:\n{stdout}"
    );
}

/// Control leg: WITHOUT monitor mode the job shares the shell's
/// process group (GNU leaves it there; no group is named by its pid).
#[test]
fn no_monitor_background_job_shares_shell_group() {
    let (stdout, stderr, code) = run_rubash(&format!(
        "sleep 30 &\n\
         pid=$!\n\
         {PGID_POLL}\n\
         own=$(ps -o pgid= -p $$ 2>/dev/null); own=${{own//[[:space:]]/}}\n\
         [ \"$pgid\" = \"$own\" ] && echo SHARED-WITH-SHELL || echo \"OTHER pgid=$pgid own=$own\"\n\
         kill -TERM -- -$pid 2>/dev/null && echo GROUPKILL-OK || echo GROUPKILL-ESRCH\n\
         kill -9 \"$pid\" 2>/dev/null\n\
         wait \"$pid\" 2>/dev/null; echo DONE"
    ));
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains("SHARED-WITH-SHELL"),
        "no-monitor job shares the shell group:\n{stdout}"
    );
    assert!(
        stdout.contains("GROUPKILL-ESRCH"),
        "no group is named by the job pid without job control:\n{stdout}"
    );
}

/// Monitor mode: an async compound body (respawned shell image) is
/// grouped too — the spawn site differs from a simple external
/// command but the jobs.c invariant is the same.
#[test]
fn monitor_compound_job_leads_own_group() {
    let (stdout, stderr, code) = run_rubash(&format!(
        "set -m\n\
         ( sleep 30 ) &\n\
         pid=$!\n\
         {PGID_POLL}\n\
         [ \"$pgid\" = \"$pid\" ] && echo GROUPED || echo SHARED\n\
         kill -TERM -- -$pid 2>/dev/null && echo GROUPKILL-OK || echo GROUPKILL-ESRCH\n\
         kill -9 -- -$pid 2>/dev/null; kill -9 \"$pid\" 2>/dev/null\n\
         wait \"$pid\" 2>/dev/null; echo \"wait rc=$?\""
    ));
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains("GROUPED"),
        "compound job must lead its group:\n{stdout}"
    );
    assert!(
        stdout.contains("GROUPKILL-OK"),
        "builtin group kill must find the compound job's group:\n{stdout}"
    );
}

/// Monitor mode: a coproc is an async child and leads its own group
/// like any other job.
#[test]
fn monitor_coproc_leads_own_group() {
    let (stdout, stderr, code) = run_rubash(
        "set -m\n\
         coproc sleep 30\n\
         pid=$COPROC_PID\n\
         pgid=$(ps -o pgid= -p \"$pid\" 2>/dev/null); pgid=${pgid//[[:space:]]/}\n\
         echo \"pid=$pid pgid=$pgid\"\n\
         [ \"$pgid\" = \"$pid\" ] && echo GROUPED || echo SHARED\n\
         kill -9 \"$pid\" 2>/dev/null\n\
         wait \"$pid\" 2>/dev/null; echo DONE",
    );
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains("GROUPED"),
        "coproc must lead its group:\n{stdout}"
    );
}

fn wait_child_bounded(child: &mut Child, limit: Duration) -> Option<std::process::ExitStatus> {
    let start = Instant::now();
    loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => return Some(status),
            None => {
                if start.elapsed() > limit {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

/// The bats CTRL-C geometry end to end (Linux: setsid provides the
/// group leadership the fixed monitor-mode spawn also produces): a
/// script shell with an INT trap blocked in a foreground `sleep`
/// receives SIGINT addressed to its whole process group from a
/// process outside the group. GNU: the real sleep dies, the trap
/// runs at once, the script exits 130. Baseline rubash: the emulated
/// sleep defers the trap for its full remainder, so the script is
/// still alive seconds later (and bats' driver kills the run).
#[cfg(target_os = "linux")]
#[test]
fn group_sigint_during_emulated_sleep_runs_trap_promptly() {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue2241-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let nested = dir.join("nested.sh");
    std::fs::write(
        &nested,
        "LATCH_DIR=\"$1\"\n\
         on_int() { echo \"NESTED: INT trap ran\"; exit 130; }\n\
         trap on_int INT\n\
         echo ready >>\"$LATCH_DIR/latch\"\n\
         sleep 10\n\
         echo \"NESTED: sleep finished naturally\"\n",
    )
    .expect("write nested script");

    let mut child = Command::new("setsid")
        .arg(env!("CARGO_BIN_EXE_rubash"))
        .arg(&nested)
        .arg(&dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn nested under setsid");
    let pid = child.id();

    // Bounded latch wait: the nested signals it is inside the sleep.
    let latch = dir.join("latch");
    let start = Instant::now();
    while !latch.exists() {
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "nested never signaled readiness"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(300));

    // The sender models bats-exec-file: a LONG-LIVED driver shell
    // whose builtin kill delivers to the group and which then waits
    // on the run. (A one-shot kill binary is not equivalent: it exits
    // before the receiver adjudicates the delivery, and its sender
    // group becomes unobservable — the receiver then keeps the
    // rubash#1034 deferral, as it must for a sender it cannot place.)
    let mut sender = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(format!("kill -INT -- -{pid}; sleep 5"))
        .spawn()
        .expect("spawn sender shell");

    let sent_at = Instant::now();
    let status = wait_child_bounded(&mut child, Duration::from_secs(8));
    let elapsed = sent_at.elapsed();
    let _ = sender.kill();
    let _ = sender.wait();
    if status.is_none() {
        let _ = Command::new("kill")
            .args(["-s", "KILL", "--", &format!("-{pid}")])
            .output();
        let _ = child.kill();
        panic!("nested still alive {elapsed:?} after group SIGINT (deferral face)");
    }
    let status = status.unwrap();
    let mut out = String::new();
    if let Some(mut so) = child.stdout.take() {
        let _ = so.read_to_string(&mut out);
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(status.code(), Some(130), "nested output:\n{out}");
    assert!(
        out.contains("NESTED: INT trap ran"),
        "INT trap must run promptly on group delivery:\n{out}"
    );
    assert!(
        !out.contains("sleep finished naturally"),
        "the sleep must be cut short by the delivery:\n{out}"
    );
    assert!(
        elapsed < Duration::from_secs(8),
        "trap dispatch must be prompt, not deferred ({elapsed:?})"
    );
}
