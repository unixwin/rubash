//! Issue rubash#2053: coproc reap vs `wait $COPROC_PID` race — after
//! the coproc's fd was read to EOF, `wait $COPROC_PID` intermittently
//! reported 127 (and, in the status-7 probe form, 0) instead of the
//! child's real exit status.
//!
//! Root cause: GNU keeps the reap and the disposal in separate hands.
//! waitchld records the status asynchronously (jobs.c:4173 waitchld
//! -> execute_cmd.c:2326 coproc_setstatus saves c_status and flags
//! COPROC_DEAD; the comment there explicitly defers disposal because
//! the handler context cannot unset variables), and coproc_reap
//! (execute_cmd.c:2225) disposes only an already-reaped coproc, from
//! cleanup_dead_jobs (jobs.c:1303) at specific disposal points: the
//! notify_and_cleanup inside a foreground wait_for (jobs.c:3385),
//! stop_pipeline after a command that forked (execute_cmd.c:963),
//! the REAP() after each loop iteration (execute_cmd.c:2975-2983,
//! call sites :3105/:3267/:3606/:3838), the wait/jobs builtins, and
//! stream end. Rubash's boundary refresh instead discovered a coproc
//! death with its own try_wait and retired the coproc (unbind
//! <name>/<name>_PID, close endpoints) in the same motion, at ANY
//! command boundary — including the boundary immediately before a
//! `wait` expanded its operand. The operand vanished mid-flight:
//! `wait` degenerated to a bare wait (status 0) or reported 127 for
//! a pid whose recorded status was never consulted.
//!
//! The fix separates the two steps in the executor: a completion
//! RECORDS the status at observation points exactly like other jobs,
//! while the retirement is deferred to GNU's disposal points (a
//! refresh that observed a foreground wait, a loop REAP point, the
//! jobs listing, an observed wait/fg refresh, stream end).
//!
//! GNU invariant these pins assert (verified against GNU bash
//! 5.3.20, 200-round probe batteries): whenever <name>_PID is still
//! bound when `wait` expands, wait returns the child's REAL status;
//! whenever it was already disposed, the operand vanishes and the
//! bare wait returns 0. The forbidden outcome is BOUND with any
//! other status. Which class a given round lands in is scheduling;
//! the agreement between the two is not.

use std::process::{Command, Stdio};

fn rubash_script(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-2053-{}",
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

/// The issue's shape, run for 100 rounds inside one script with a
/// distinctive child status (7): every round must satisfy the GNU
/// class invariant — BOUND rounds report the recorded status 7,
/// UNSET rounds (coproc already disposed at a real disposal point)
/// report the bare-wait 0. A BOUND round reporting anything else is
/// the #2053 steal.
#[test]
fn wait_after_coproc_eof_honours_recorded_status_100_rounds() {
    let (stdout, stderr, _) = rubash_script(
        "violations=0\n\
         for r in {1..100}; do\n\
         \x20 (\n\
         \x20   coproc C { exit 7; }\n\
         \x20   exec {C[1]}>&-\n\
         \x20   cat <&${C[0]}\n\
         \x20   if [ -n \"${C_PID:+x}\" ]; then s=BOUND; else s=UNSET; fi\n\
         \x20   wait $C_PID\n\
         \x20   rc=$?\n\
         \x20   if [ \"$s\" = BOUND ] && [ \"$rc\" -ne 7 ]; then echo \"VIOLATION bound rc=$rc\"; fi\n\
         \x20   if [ \"$s\" = UNSET ] && [ \"$rc\" -ne 0 ]; then echo \"VIOLATION unset rc=$rc\"; fi\n\
         \x20 )\n\
         done\n\
         echo \"rounds done\"\n",
    );
    assert_eq!(stdout, "rounds done\n", "stderr: {stderr}");
}

/// No disposal point intervenes between the child's death and the
/// wait (a blocking `read` builtin is not one — GNU bash 5.3.20,
/// 30/30 rounds): the pid variable must still be bound when the
/// probe expands, and wait must return the recorded status 7, not
/// the stolen-operand 0 the pre-fix boundary retirement produced.
#[test]
fn wait_after_blocking_builtin_gets_recorded_status() {
    let (stdout, stderr, _) = rubash_script(
        "coproc C { exit 7; }\n\
         read -t 0.5 < /dev/null\n\
         echo \"S=${C_PID:+BOUND}\"\n\
         wait $C_PID\n\
         echo \"rc=$?\"\n",
    );
    assert_eq!(stdout, "S=BOUND\nrc=7\n", "stderr: {stderr}");
}

/// The other side of the cadence (guard against over-deferral): a
/// loop is a GNU disposal point (REAP() per iteration), so after a
/// builtin-only loop the dead coproc's variables are gone — the
/// #1023 contract at loop points. The loop is wall-clock bounded
/// (one SECONDS tick of pure builtins) so the child's exit is
/// recorded mid-loop on any build profile or machine speed.
#[test]
fn dead_coproc_vars_disposed_after_loop_iterations() {
    let (stdout, stderr, _) = rubash_script(
        "coproc C { exit 7; }\n\
         t=$SECONDS\n\
         while (( SECONDS == t )); do :; done\n\
         echo \"S=${C_PID:+BOUND}\"\n",
    );
    assert_eq!(stdout, "S=\n", "stderr: {stderr}");
}

/// And after a real external command (a foreground wait point — the
/// emulated `sleep` fast path counts, it stands in for GNU's child),
/// disposal has landed before the next expansion.
#[test]
fn dead_coproc_vars_disposed_after_foreground_wait() {
    let (stdout, stderr, _) = rubash_script(
        "coproc C { exit 7; }\n\
         sleep 1\n\
         echo \"S=${C_PID:+BOUND}\"\n",
    );
    assert_eq!(stdout, "S=\n", "stderr: {stderr}");
}
