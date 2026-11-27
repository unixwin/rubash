//! Issue rubash#3948 — coproc retirement must be keyed to GNU's reap
//! (disposal) points, never to a `read` hitting EOF, and must happen at
//! every real reap point on every platform.
//!
//! GNU separates recording from disposal (rubash#2053/#2057): waitchld
//! only marks the coproc dead (jobs.c:4173 coproc_pidchk ->
//! execute_cmd.c:2323 coproc_setstatus sets COPROC_DEAD — the comment
//! at :2329 explicitly defers disposal out of signal-handler context),
//! while coproc_reap (execute_cmd.c:2225 -> coproc_dispose :2166 ->
//! coproc_unsetvars :2450 + coproc_close :2199) runs only from
//! cleanup_dead_jobs (jobs.c:1303), whose callers are wait_for's
//! notify_and_cleanup (jobs.c:3385), the loop REAP() points, the
//! wait/jobs builtins, and stream end. A blocking `read` builtin is
//! none of those: `read -u ${COPROC[0]}` past EOF on an exited-but-
//! unreaped coproc returns rc 1 with NO diagnostic, COPROC/COPROC_PID
//! stay bound, and a later explicit close succeeds.
//!
//! The Windows face fixed here: the sliced foreground wait in
//! wait_external_child (external_inner.rs `#[cfg(not(unix))]` arm)
//! never called note_foreground_wait, while the unix arm notes the
//! same block (jobs.c:3064 wait_for). A real spawned command was
//! therefore no disposal point on Windows: `coproc { printf x; };
//! /bin/true; read -u ${COPROC[0]}` kept COPROC bound where GNU's
//! wait_for disposes it inside the external command's own wait.
//!
//! All expectations verified against GNU Bash 5.3.20 (-c probes: one
//! command list, so no intermediate read_command cleanup_dead_jobs
//! boundary separates the reads — the deterministic form the issue's
//! seed uses).

use std::process::{Command, Stdio};

fn rubash_c(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The issue's primary face: `read -u` past EOF on a coproc whose
/// child has exited but not been reaped returns rc 1 silently, and
/// COPROC[0] stays bound — no disposal point has passed (a builtin
/// read only hands GNU's SIGCHLD handler a reap-recording window;
/// disposal still waits for a real reap point). GNU 5.3.20 prints no
/// diagnostic and keeps `fd0=set`.
#[test]
fn eof_reads_on_exited_coproc_do_not_retire_it() {
    let (stdout, stderr, _) = rubash_c(
        "coproc { printf 'one\ntwo\n'; }; exec {COPROC[1]}>&-; \
         read -u ${COPROC[0]} a; echo \"r1=$? a=[$a]\"; \
         read -u ${COPROC[0]} b; echo \"r2=$? b=[$b]\"; \
         read -u ${COPROC[0]} c; echo \"r3=$? c=[$c]\"; \
         read -u ${COPROC[0]} d; echo \"r4=$? d=[$d]\"; \
         if [ -n \"${COPROC[0]:-}\" ]; then echo fd0=set; else echo fd0=unset; fi",
    );
    assert_eq!(
        stdout, "r1=0 a=[one]\nr2=0 b=[two]\nr3=1 c=[]\nr4=1 d=[]\nfd0=set\n",
        "stderr: {stderr}"
    );
    assert_eq!(stderr, "", "stderr: {stderr}");
}

/// The redirect form of the same face (`read <&${COPROC[0]}`): same
/// EOF silence, same bound array (GNU 5.3.20 identical).
#[test]
fn redirect_form_eof_reads_do_not_retire_it() {
    let (stdout, stderr, _) = rubash_c(
        "coproc { printf 'one\ntwo\n'; }; \
         read a <&${COPROC[0]}; echo \"r1=$? a=[$a]\"; \
         read b <&${COPROC[0]}; echo \"r2=$? b=[$b]\"; \
         read c <&${COPROC[0]}; echo \"r3=$? c=[$c]\"; \
         if [ -n \"${COPROC[0]:-}\" ]; then echo fd0=set; else echo fd0=unset; fi",
    );
    assert_eq!(
        stdout, "r1=0 a=[one]\nr2=0 b=[two]\nr3=1 c=[]\nfd0=set\n",
        "stderr: {stderr}"
    );
    assert_eq!(stderr, "", "stderr: {stderr}");
}

/// The array surviving the EOF reads means a later explicit close
/// still resolves its operand: GNU answers `exec {COPROC[0]}<&-`
/// with rc 0 (verified 5.3.20).
#[test]
fn explicit_close_succeeds_after_eof_reads() {
    let (stdout, stderr, _) = rubash_c(
        "coproc { printf 'x\n'; }; exec {COPROC[1]}>&-; \
         read -u ${COPROC[0]} a; read -u ${COPROC[0]} b; \
         exec {COPROC[0]}<&-; echo \"close=$?\"",
    );
    assert_eq!(stdout, "close=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stderr: {stderr}");
}

/// The complementary face and the #3948 fix itself: a real spawned
/// foreground command's wait IS a disposal point (GNU jobs.c:3064
/// wait_for -> jobs.c:3385 notify_and_cleanup -> coproc_reap), on
/// Windows too. `$THIS_SH -c 'sleep 0.5'` is a real child whose wait
/// dwarfs the coproc's own spawn+exit latency on any machine, so the
/// `printf` child is reaped inside it deterministically: COPROC is
/// unbound before the next command expands, and `read -u` then fails
/// with GNU's invalid-fd diagnostic.
#[test]
fn external_command_wait_retires_dead_coproc() {
    let (stdout, stderr, _) = rubash_c(
        "coproc { printf 'x\n'; }; exec {COPROC[1]}>&-; \
         \"$THIS_SH\" -c 'sleep 0.5'; \
         echo \"S=${COPROC_PID:+BOUND} arr=[${COPROC[@]:-unset}]\"; \
         read -u ${COPROC[0]} a; echo \"r=$? a=[$a]\"",
    );
    assert_eq!(stdout, "S= arr=[unset]\nr=1 a=[]\n", "stderr: {stderr}");
    assert!(
        stderr.contains("invalid file descriptor specification"),
        "stderr: {stderr}"
    );
}
