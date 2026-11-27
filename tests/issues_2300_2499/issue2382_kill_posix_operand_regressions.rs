//! POSIX mode and the kill operand loop (rubash#2382; GNU bash
//! 5.3.20 oracle-verified 2026-10-31 — the filed premise was
//! FALSIFIED, these pins assert the behavior GNU actually has).
//!
//! The claim was that posix mode stops the operand loop at the first
//! failure (CONTINUE_OR_FAIL). The source says the opposite:
//! builtins/kill.def:81-85 maps CONTINUE_OR_FAIL to
//! `goto continue_killing` because config-top.h:21-24 DEFINES
//! CONTINUE_AFTER_KILL_ERROR ("This is what POSIX.2 specifies" — the
//! operands are all processed). POSIX mode changes only the final
//! tally (kill.def:283-286): posix fails when any attempt failed
//! (any_failed), the default mode fails only when nothing succeeded
//! (!any_succeeded). Verified against the oracle by strace in all
//! three posix entry modes (`set -o posix`, `--posix`,
//! POSIXLY_CORRECT=1): after `kill(999999, SIGTERM) = -1 ESRCH` the
//! oracle still issues `kill(<live>, SIGTERM) = 0`.
//!
//! Aliveness is asserted through the target's wait status behind a
//! watchdog (128+TERM if the tested kill delivered, 128+KILL from
//! the watchdog if it never did) — never through `kill -0`, which
//! answers success for an unreaped zombie and was the source of the
//! original misreading.

#![cfg(unix)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn rubash_bin() -> String {
    env!("CARGO_BIN_EXE_rubash").to_string()
}

/// Run a script FILE under the built rubash; return (stdout, stderr).
fn run_script(tag: &str, body: &str) -> (String, String) {
    let dir = std::env::temp_dir().join(format!("issue2382-{tag}-{}", std::process::id()));
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

/// Collect the target's wait status behind a watchdog: if the
/// tested kill delivered TERM, `wait` returns 128+TERM (143) at
/// once; if the operand was never processed (the falsified
/// fail-fast behavior), the target is still asleep and only the
/// watchdog's KILL ends the wait, reporting 128+KILL (137). The
/// distinction is decisive and bounded (~2 s worst case).
const OBSERVE_DEATH: &str = "( sleep 2; kill -KILL $live 2>/dev/null ) >/dev/null 2>&1 &\n\
     wait $live; echo \"wait rc=$?\"\n";

#[test]
fn issue2382_posix_processes_operands_after_a_failure() {
    let body = format!(
        "set -o posix\n\
         sleep 60 & live=$!\n\
         kill 999999 $live; echo \"rc=$?\"\n\
         {OBSERVE_DEATH}"
    );
    let (stdout, stderr) = run_script("posix-continues", &body);
    assert_eq!(stdout, "rc=1\nwait rc=143\n");
    assert!(
        stderr.contains("(999999) - No such process"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue2382_nonposix_processes_operands_after_a_failure() {
    let body = format!(
        "sleep 60 & live=$!\n\
         kill 999999 $live; echo \"rc=$?\"\n\
         {OBSERVE_DEATH}"
    );
    let (stdout, stderr) = run_script("nonposix-continues", &body);
    assert_eq!(stdout, "rc=0\nwait rc=143\n");
    assert!(
        stderr.contains("(999999) - No such process"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue2382_posix_failure_after_the_live_operand_still_fails() {
    // Operand order does not matter to the posix tally: the live
    // target is signaled (and dies), the bad pid fails the command.
    let body = format!(
        "set -o posix\n\
         sleep 60 & live=$!\n\
         kill $live 999999; echo \"rc=$?\"\n\
         {OBSERVE_DEATH}"
    );
    let (stdout, stderr) = run_script("posix-order", &body);
    assert_eq!(stdout, "rc=1\nwait rc=143\n");
    assert!(
        stderr.contains("(999999) - No such process"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue2382_all_bad_operands_fail_in_both_modes() {
    let (stdout, stderr) = run_script(
        "all-bad",
        "kill 999999 999998; echo \"nonposix rc=$?\"\n\
         set -o posix\n\
         kill 999999 999998; echo \"posix rc=$?\"\n",
    );
    assert_eq!(stdout, "nonposix rc=1\nposix rc=1\n");
    // Each invocation reports both bad pids.
    assert_eq!(
        stderr.matches("(999999) - No such process").count(),
        2,
        "stderr: {stderr}"
    );
    assert_eq!(
        stderr.matches("(999998) - No such process").count(),
        2,
        "stderr: {stderr}"
    );
}

#[test]
fn issue2382_posix_spec_diagnostic_feeds_neither_counter() {
    // kill.def: a jobspec diagnostic (no such job) touches neither
    // any_failed nor any_succeeded, so in posix mode a bad spec in
    // front of a good pid still returns 0 — and the pid is still
    // processed (no fail-fast for spec diagnostics either).
    let body = format!(
        "set -o posix\n\
         sleep 60 & live=$!\n\
         kill %nosuchjob $live; echo \"rc=$?\"\n\
         {OBSERVE_DEATH}"
    );
    let (stdout, stderr) = run_script("posix-spec-diag", &body);
    assert_eq!(stdout, "rc=0\nwait rc=143\n");
    assert!(
        stderr.contains("kill: %nosuchjob: no such job"),
        "stderr: {stderr}"
    );
}
