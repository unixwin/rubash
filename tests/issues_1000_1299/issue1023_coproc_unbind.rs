//! Issue rubash#1023: two simultaneous named coprocs — GNU tears a dead
//! coproc's `NAME` / `NAME_PID` variables down at the first reap point
//! after its exit, but rubash's per-command boundary refresh kept them
//! whenever the upcoming command's redirect referenced the coproc name
//! (`coprocs_referenced_by_command` / `protected_coprocs`). GNU has no
//! such protection: cleanup_dead_jobs -> coproc_reap (jobs.c:1342)
//! disposes every COPROC_DEAD entry through coproc_dispose ->
//! coproc_unsetvars (execute_cmd.c:2166, :2450), so `cat <&${A[0]}` for
//! a reaped coproc expands nothing and reports `ambiguous redirect`.
//! A coproc still running at the boundary is not dead and keeps its
//! variables — GNU never tears down a live coproc either.
//!
//! GNU references:
//! - jobs.c:1342 cleanup_dead_jobs -> coproc_reap, reached from
//!   notify_and_cleanup (jobs.c:3642) at each input-line read and from
//!   stop_pipeline (jobs.c:572) after every spawned pipeline.
//! - execute_cmd.c:2166 coproc_dispose -> :2450 coproc_unsetvars:
//!   unbind `<name>_PID` then check_unbind_variable on `<name>`.
//!
//! Baselines below were verified against WSL GNU bash 5.3.20.

#![cfg(windows)]

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn run_script(script: &str) -> (Option<i32>, String, String) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1023-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue1023 scratch dir");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash script");
    let _ = std::fs::remove_dir_all(&dir);
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The issue's reproducer with the spawn race removed (a `sleep` lets both
/// coproc children exit before the reads): GNU unbinds both arrays, so
/// each `<&${A[0]}` reports ambiguous redirect and the script's own
/// commands keep running.
#[test]
fn two_dead_named_coprocs_unbind_both_var_sets() {
    // WSL GNU 5.3.20:
    //   line 4: ${A1[0]}: ambiguous redirect
    //   line 5: ${A2[0]}: ambiguous redirect
    //   done rc=1
    let (_, stdout, stderr) = run_script(
        "coproc A1 { echo one; }\ncoproc A2 { echo two; }\nsleep 1\ncat <&${A1[0]}\ncat <&${A2[0]}\necho \"done rc=$?\"\n",
    );
    assert_eq!(stdout, "done rc=1\n");
    assert!(
        stderr.contains("${A1[0]}: ambiguous redirect"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("${A2[0]}: ambiguous redirect"),
        "stderr: {stderr}"
    );
}

/// A single dead coproc referenced by the next command's redirect is
/// unbound too — GNU's reap consults nothing about the upcoming words
/// (the protection that used to keep it is the bug).
#[test]
fn dead_coproc_referenced_by_redirect_is_unbound() {
    // WSL GNU 5.3.20: `declare: A1: not found` / `declare: A1_PID: not
    // found`, rc 1 — both names gone once the child has exited.
    let (_, stdout, stderr) =
        run_script("coproc A1 { echo one; }\nsleep 1\ndeclare -p A1 A1_PID\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("declare: A1: not found"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("declare: A1_PID: not found"),
        "stderr: {stderr}"
    );
}

/// A coproc still running at the boundary keeps its variables and its
/// pipes — GNU disposes only COPROC_DEAD entries.
#[test]
fn live_coproc_keeps_vars_and_roundtrips() {
    // WSL GNU 5.3.20: the reflector answers and the vars stay bound.
    let (_, stdout, _) = run_script(
        "coproc R { cat; }\necho hi >&${R[1]}\nread LINE <&${R[0]}\necho \"got=$LINE bound=${#R[@]} pid=${R_PID:+set}\"\nkill $R_PID 2>/dev/null\nwait 2>/dev/null\n",
    );
    assert_eq!(stdout, "got=hi bound=2 pid=set\n");
}
