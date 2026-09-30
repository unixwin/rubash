//! Issue rubash#352 regressions: coproc variable lifecycle on readonly
//! names and on reap — locks the byte-parity with GNU Bash 5.3.0 verified
//! on 2026-09-28 (the readonly repro itself was already passing on base
//! 0914200e; this suite pins it).
//!
//! GNU model (vendored source):
//! - execute_cmd.c:2450 coproc_unsetvars(): on dispose,
//!   unbind_variable_noref("<name>_PID") removes the _PID variable with
//!   NO readonly gate (a readonly RO_PID disappears), then
//!   check_unbind_variable(c_name) refuses readonly targets with
//!   "name: cannot unset: readonly variable".
//! - The dispose runs at coproc_reap (execute_cmd.c:2225 cpl_reap ->
//!   coproc_dispose), reached from cleanup_dead_jobs (jobs.c:1342 /
//!   nojobs.c:392) — which runs at stop_pipeline (jobs.c:572), the REAP()
//!   loop-body points (execute_cmd.c:2975), and the wait paths.
//! - MULTIPLE_COPROCS is 1 by default (config-top.h:143): cpl_reap
//!   disposes EVERY dead coproc entry, and coproc_unsetvars unbinds by
//!   NAME — so a dead coproc's dispose removes a freshly-bound
//!   same-name successor's variables. That is observable as: after
//!   `coproc X { :; }; <wait-ish delay>; coproc X { ... }`, $X_PID is
//!   unset by the next command boundary.
//!
//! rubash mirrors: executor/job_builtins.rs coproc_unset_vars() runs at
//! every GNU-equivalent poll point (per-command pre-refresh =
//! eval.c:355 notify_and_cleanup, REAP loop bodies, wait/jobs builtins,
//! foreground-external completion).
//!
//! Timing note (platform-bound, documented not "fixed"): a coproc child
//! that has not exited by the observation point keeps its variables in
//! BOTH shells — `coproc X { sleep .3; }; coproc X { sleep .3; }; echo
//! $X_PID` shows the pid under GNU 5.3.0 too (verified). The shapes
//! where GNU's fork-fast children make the unbind observable with no
//! separator are a child-exit-latency race on Windows (~30ms process
//! spawn) and are intentionally NOT pinned here.
//!
//! All expected outputs byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script files 2026-09-28, matrix under
//! target/issue-suites/results/resid21/i352/ in the lane worktree).

use std::io::Write;
use std::process::{Command, Stdio};

fn rubash_stdin(script: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash stdin");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(script.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("collect stdin run");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The issue's exact reproducer. GNU 5.3.0 (byte-verified): rc=1, stdout
/// `declare -r RO_PID` (line 3: RO_PID is the user's own readonly var and
/// still listed) then `declare -r RO` (line 5: only RO — the reap during
/// `wait` removed RO_PID via unbind_variable_noref despite readonly, and
/// RO itself survived with "cannot unset: readonly variable").
#[test]
fn readonly_coproc_name_pid_lifecycle_matches_gnu() {
    let (stdout, stderr, code) = rubash_stdin(
        "declare -r RO RO_PID\ncoproc RO { :; }\ndeclare -p RO_PID\nwait\ndeclare -p RO RO_PID\n",
    );
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "declare -r RO_PID\ndeclare -r RO\n");
    assert!(
        stderr.contains("line 2: RO: readonly variable"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("line 4: RO: cannot unset: readonly variable"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("line 5: declare: RO_PID: not found"),
        "stderr: {stderr}"
    );
}

/// A live coproc keeps X_PID bound; the reap at `wait` unbinds both X and
/// X_PID (GNU coproc_dispose at wait time).
#[test]
fn live_coproc_pid_bound_until_wait_reap() {
    let (stdout, stderr, code) = rubash_stdin(
        "coproc X { sleep 1; echo hi; }\ndeclare -p X_PID >/dev/null 2>&1 && echo immediate-bound\nwait\ndeclare -p X_PID >/dev/null 2>&1 || echo post-wait-gone\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "immediate-bound\npost-wait-gone\n");
}

/// $X_PID carries the coproc pid while alive; after `wait` it is unset and
/// `declare -p X` reports not-found (GNU: coproc_unsetvars unbinds both).
#[test]
fn pid_value_and_array_unbind_after_wait() {
    let (stdout, stderr, code) = rubash_stdin(
        "coproc X { sleep 1; }\necho \"pid-shape=[$(echo $X_PID | grep -c '^[0-9]*[0-9]$')]\"\nwait\necho \"post=[$X_PID]\"\ndeclare -p X 2>&1 | tail -1\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let expected_tail = "declare: X: not found";
    assert!(
        stdout.starts_with("pid-shape=[1]\npost=[]\n")
            && stdout.trim_end().ends_with(expected_tail),
        "stdout: {stdout}"
    );
}

/// Restart with a separator (the deterministic window): the dead first
/// coproc's dispose at the separator's completion unbinds by NAME, so the
/// second coproc's binding is in place at q1 and gone after the next
/// reap point (q2) — GNU 5.3.0 byte-verified q1=[pid] q2=[] q3=[].
#[test]
fn restart_with_separator_pid_visibility_matches_gnu() {
    let (stdout, stderr, code) = rubash_stdin(
        "coproc X { :; }\nsleep 0.4\ncoproc X { :; }\necho \"q1-set=[$(echo $X_PID | grep -c '^[0-9]*[0-9]$')]\"\nsleep 0.4\necho \"q2=[$X_PID]\"\nwait\necho \"q3=[$X_PID]\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "q1-set=[1]\nq2=[]\nq3=[]\n");
}

/// Both coprocs alive at the observation point: X_PID shows the second
/// coproc's pid under GNU as well (platform timing, not a semantic gap).
#[test]
fn both_alive_restart_shows_second_pid() {
    let (stdout, stderr, code) = rubash_stdin(
        "coproc X { sleep 0.5; }\ncoproc X { sleep 0.5; }\necho \"both-set=[$(echo $X_PID | grep -c '^[0-9]*[0-9]$')]\"\nwait\necho \"after=[$X_PID]\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "both-set=[1]\nafter=[]\n");
}
