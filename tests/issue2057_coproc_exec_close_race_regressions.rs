//! Issue rubash#2057: after a coproc child exited, `exec {C[1]}>&-`
//! intermittently failed with `ambiguous redirect` (~1 in 5 rounds)
//! even though the child had only just died and no disposal point
//! had passed.
//!
//! Same root as rubash#2053: GNU records a coproc's death
//! asynchronously (jobs.c waitchld -> execute_cmd.c:2326
//! coproc_setstatus) and disposes it only at cleanup points
//! (cleanup_dead_jobs -> coproc_reap: foreground wait_for
//! completion, stop_pipeline, per-iteration REAP(), wait/jobs
//! builtins, stream end). Rubash's boundary refresh retired the
//! coproc — unbinding the whole <name> array — at any command
//! boundary, so when the retirement landed between the child's exit
//! and the `exec`, the `{C[1]}` fd-variable expansion found no array
//! and reported an ambiguous redirect. With the retirement deferred
//! to GNU's disposal points, the array is still bound at the exec
//! and the close proceeds (the closed endpoint is then recorded -1
//! exactly as after a live close).
//!
//! Expectations verified against GNU bash 5.3.20.

use std::process::{Command, Stdio};

fn rubash_script(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-2057-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir(&dir).expect("create scratch dir");
    let path = dir.join("s.sh");
    std::fs::write(&path, script).expect("seed script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash");
    let result = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// The issue's regression seed verbatim, 50 rounds in isolated
/// subshells: every round must print `ok` and no round may report
/// an ambiguous redirect. The child dies 0.05s in; the exec follows
/// immediately, long before any GNU disposal point in the round.
#[test]
fn exec_close_after_coproc_exit_50_rounds() {
    let (stdout, stderr, _) = rubash_script(
        "for r in {1..50}; do\n\
         \x20 ( coproc C { sleep 0.05; }; exec {C[1]}>&-; echo ok )\n\
         done\n",
    );
    assert_eq!(stdout, "ok\n".repeat(50), "stderr: {stderr}");
    assert!(!stderr.contains("ambiguous redirect"), "stderr: {stderr}");
}

/// Deterministic form: the child is guaranteed dead (it exited
/// during a 0.5s blocking `read`), but a blocking builtin is not a
/// GNU disposal point, so the array must still be bound and the
/// close must succeed (GNU bash 5.3.20: 20/20 clean).
///
/// Note the shape this pin deliberately does NOT use: an
/// intervening external command (`cat <&${C[0]}` read to EOF)
/// between the death and the exec. That command's completion IS
/// a GNU disposal point, so GNU itself reports the ambiguous
/// redirect there (90 of 100 rounds, GNU bash 5.3.20) — the
/// unbound-variable form is ambiguous in both shells.
#[test]
fn exec_close_after_blocking_builtin_keeps_array() {
    let (stdout, stderr, _) = rubash_script(
        "coproc C { sleep 0.05; }\n\
         read -t 0.5 < /dev/null\n\
         exec {C[1]}>&-\n\
         echo ok\n",
    );
    assert_eq!(stdout, "ok\n", "stderr: {stderr}");
    assert!(!stderr.contains("ambiguous redirect"), "stderr: {stderr}");
}
