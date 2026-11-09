//! rubash#2695 — coproc death is not noticed until `wait`.
//!
//! GNU ground truth (bash 5.3.20 oracle, probed from script files):
//! the SIGCHLD reap path marks the coproc dead — waitchld
//! (jobs.c:4172-4173) calls coproc_pidchk, whose coproc_setstatus
//! (execute_cmd.c:2318ff) sets COPROC_DEAD — and the job-status
//! sweep retires it: coproc_reap (execute_cmd.c:2225) disposes it
//! (coproc_unsetvars + coproc_close), called from the sweep at
//! jobs.c:1342 and from the wait_for_job path at jobs.c:3494. So
//! once the shell has had any command boundary after the death,
//! ${COPROC[0]} expands empty and a `read -u ${COPROC[0]}` fails
//! with read.def:373's "invalid file descriptor specification"
//! instead of reading EOF from the stale fd.
//!
//! rubash retires the coproc only on the explicit `wait` path
//! (external_finish.rs); its notify sweep leaves COPROC/COPROC_PID
//! set and the stale fd readable-as-EOF.
//!
//! Determinism: the bare two-read shape is timing-sensitive even
//! in GNU (the audit saw it flip); these cells put a real external
//! command boundary (a foreground wait, which sweeps dead children
//! in both shells' designs) between the death and the check, which
//! makes the GNU side deterministic.

#![cfg(unix)]

use std::io::Write;
use std::process::{Command, Stdio};

fn run_script(body: &str) -> (String, String, Option<i32>) {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rubash-issue2695-{}-{}", std::process::id(), seq));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let script = dir.join("case.sh");
    let mut f = std::fs::File::create(&script).expect("write script");
    f.write_all(body.as_bytes()).expect("write body");
    drop(f);
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script)
        .stdin(Stdio::null())
        .output()
        .expect("spawn rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// After the coproc's output is consumed (it has exited) and one
/// foreground external command has run, the coproc is retired:
/// the second read fails on the empty expansion, `b` stays unset,
/// and both COPROC and COPROC_PID are gone.
#[test]
fn dead_coproc_retired_before_second_read() {
    let (stdout, stderr, code) = run_script(
        "coproc { echo only; }\n\
         read -u ${COPROC[0]} a\n\
         /bin/sleep 0.3\n\
         read -u ${COPROC[0]} b\n\
         echo \"rc=$? a=[$a] b=[${b-unset}] c0=[${COPROC[0]-empty}] cpid=[${COPROC_PID-empty}]\"\n",
    );
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(
        stdout, "rc=1 a=[only] b=[unset] c0=[empty] cpid=[empty]\n",
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("invalid file descriptor specification"),
        "the retired coproc's read must fail as a bad fd: {stderr}"
    );
}

/// State-only face of the same mechanism: an EOF read proves the
/// coproc is dead; after one more command boundary the COPROC
/// variables must already be retired (no `wait` involved).
#[test]
fn dead_coproc_state_retired_after_eof_and_boundary() {
    let (stdout, stderr, code) = run_script(
        "coproc { echo only; }\n\
         read -u ${COPROC[0]} a\n\
         read -u ${COPROC[0]} x\n\
         /bin/true\n\
         echo \"a=[$a] x=[$x] c0=[${COPROC[0]-empty}] cpid=[${COPROC_PID-empty}]\"\n",
    );
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(
        stdout, "a=[only] x=[] c0=[empty] cpid=[empty]\n",
        "stderr: {stderr}"
    );
}

/// Control (regression lock): the explicit `wait` path retires
/// the coproc in both shells — this already works in rubash and
/// must keep working.
#[test]
fn wait_still_retires_coproc_control() {
    let (stdout, stderr, code) = run_script(
        "coproc { echo only; }\n\
         wait $COPROC_PID\n\
         echo \"waitrc=$? c0=[${COPROC[0]-empty}] cpid=[${COPROC_PID-empty}]\"\n",
    );
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(
        stdout, "waitrc=0 c0=[empty] cpid=[empty]\n",
        "stderr: {stderr}"
    );
}
