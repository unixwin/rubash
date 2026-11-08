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
