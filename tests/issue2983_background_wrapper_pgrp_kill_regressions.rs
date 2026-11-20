//! rubash#2983 regressions: `kill -SIGINT -- -$PID` on a backgrounded
//! job reported `kill: (-N) - No such process` because the background
//! wrapper child died instantly — the same two defects as rubash#2982
//! (the `-c` prelude rendered `[[ $1 == "" ]]` as `[[ $1 ==  ]]`, a
//! parse error, and the functrace/errtrace-inherited DEBUG/ERR traps
//! fired while the prelude's functions did not exist yet), so the
//! process group the harness recorded was already gone by kill time.
//! GNU keeps the background child alive (execute_cmd.c:1576
//! execute_in_subshell forks the whole table; jobs.c make_child puts
//! the job in its own process group under `set -m`), so the kill
//! lands and `wait` reports 130.
//!
//! Expectations verified byte-for-byte against GNU Bash 5.3.20
//! script-file probes (self-compiled oracle).

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue2983-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The bats file_setup_teardown #8 shape, minimized: `set -m` plus a
/// background job that must take the wrapper path (fd 4 open, as
/// bats' `exec 4<&1`, plus a function table with trap handlers), then
/// SIGINT to its process group. Before the fix the child died at
/// startup and the kill failed with `No such process`; after the fix
/// the kill lands (rc=0) and `wait` reports 130.
#[test]
fn issue2983_background_job_pgrp_survives_for_group_kill() {
    let (stdout, stderr, code) = rubash_file(
        "set -eET\n\
         dbg() { local x; norm x \"$1\"; :; }\n\
         norm() { printf -v \"$1\" '%s' \"$2\"; }\n\
         err() { chk; :; }\n\
         chk() { local s=\"$?\"; if [[ -z ${DONE:-} ]]; then trap - DEBUG; fi; }\n\
         d1() { :; }; d2() { :; }; d3() { :; }; d4() { :; }; d5() { :; }; d6() { :; }\n\
         sortish() { if [[ $1 == \"\" ]]; then echo empty; else echo \"got:$1\"; fi; }\n\
         trap 'dbg \"$BASH_SOURCE\" \"$_\"' DEBUG\n\
         trap 'err \"$_\"' ERR\n\
         exec 4<&1\n\
         set -m\n\
         sleep 5 &\n\
         pid=$!\n\
         sleep 1\n\
         kill -SIGINT -- -$pid; echo \"kill rc=$?\"\n\
         wait $pid 2>/dev/null || echo \"wait rc=$?\"\n\
         echo done\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "kill rc=0\nwait rc=130\ndone\n");
}

/// The second #2983 face: the wrapper's SIMPLE-command foreground
/// wait must publish the wrapper relay target (rubash#2626 published
/// it only for the pipeline wait), or a group-delivered SIGINT kills
/// the wrapper — the relay, finding no target, restores the saved
/// disposition and re-raises — and the rubash#737 PDEATHSIG arm
/// SIGKILLs the script child before its own INT trap can run. This
/// is bats file_setup_teardown #8's teardown_file shape minimized:
/// the background job is a script child whose INT trap must run.
/// GNU (and the fixed build): the trap runs, the script continues
/// and exits 0, and `wait` reports 0; before the fix the log held
/// only the start line and `wait` reported the wrapper's own 130.
///
/// Expectations verified byte-for-byte against GNU Bash 5.3.20
/// script-file probes (self-compiled oracle) and against the
/// pre-fix master binary (kill rc=0, wait rc=130, trap line absent).
#[cfg(unix)]
#[test]
fn issue2983_wrapper_simple_wait_relays_group_sigint_to_script_child() {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue2983-relay-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let log = dir.join("trap.log");
    std::fs::write(
        dir.join("inner.sh"),
        "trap 'echo trap-ran >> \"$LOG\"' INT\n\
         echo inner started >> \"$LOG\"\n\
         sleep 30\n\
         echo completed >> \"$LOG\"\n",
    )
    .expect("write inner.sh");
    let case = "set -eET\n\
         dbg() { local x; norm x \"$1\"; :; }\n\
         norm() { printf -v \"$1\" '%s' \"$2\"; }\n\
         err() { chk; :; }\n\
         chk() { local s=\"$?\"; if [[ -z ${DONE:-} ]]; then trap - DEBUG; fi; }\n\
         d1() { :; }; d2() { :; }; d3() { :; }; d4() { :; }; d5() { :; }; d6() { :; }\n\
         sortish() { if [[ $1 == \"\" ]]; then echo empty; else echo \"got:$1\"; fi; }\n\
         trap 'dbg \"$BASH_SOURCE\" \"$_\"' DEBUG\n\
         trap 'err \"$_\"' ERR\n\
         exec 4<&1\n\
         set -m\n\
         export LOG=__LOG__\n\
         \"__BIN__\" __INNER__ &\n\
         pid=$!\n\
         n=0; while [[ ! -f \"$LOG\" && $n -lt 100 ]]; do sleep 0.1; n=$((n+1)); done\n\
         sleep 1\n\
         kill -SIGINT -- -$pid; echo \"kill rc=$?\"\n\
         wait $pid 2>/dev/null || echo \"wait rc=$?\"\n\
         echo \"LOG-BEGIN\"\n\
         cat \"$LOG\" 2>/dev/null || echo MISSING\n\
         echo \"LOG-END\"\n\
         echo done\n"
        .replace("__LOG__", &log.to_string_lossy())
        .replace("__BIN__", env!("CARGO_BIN_EXE_rubash"))
        .replace("__INNER__", &dir.join("inner.sh").to_string_lossy());
    std::fs::write(dir.join("case.sh"), case).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stderr, "");
    assert_eq!(
        stdout,
        "kill rc=0\nLOG-BEGIN\ninner started\ntrap-ran\ncompleted\nLOG-END\ndone\n"
    );
}
