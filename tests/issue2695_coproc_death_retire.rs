//! rubash#2695 — coproc death is not noticed until `wait`.
//!
//! GNU ground truth (bash 5.3.20 oracle, probed from script files):
//! the SIGCHLD reap path marks the coproc dead — waitchld
//! (jobs.c:4172-4173) calls coproc_pidchk, whose coproc_setstatus
//! (execute_cmd.c:2318ff) sets COPROC_DEAD — and the job-status
//! sweep retires it: coproc_reap (execute_cmd.c:2225) disposes it
//! (coproc_unsetvars + coproc_close), called from the sweep at
//! jobs.c:1342 and from the wait_for_job path at jobs.c:3494. The
//! retirement happens at ordinary command boundaries, builtin-only
//! sequences included. So once the shell has run anything after
//! the death, ${COPROC[0]} expands empty and a
//! `read -u ${COPROC[0]}` fails with read.def:373's "invalid file
//! descriptor specification" instead of reading EOF from the
//! stale fd.
//!
//! rubash base (f17644d7, probed): the verbatim two-read shape is
//! stale in 3/3 runs (b silently set to "", COPROC still set) —
//! the retirement exists ONLY on paths that finish an external
//! command (external_finish.rs sweeps there, the explicit `wait`
//! path included); a builtin-only sequence never retires it.
//!
//! Determinism: the bare two-read shape is timing-sensitive even
//! in GNU (the audit saw it flip; 1 of 3 oracle runs retired in
//! time). The defect cell below first reads to EOF — after which
//! the coproc's death is a certainty, not a race — then burns a
//! short builtin-only loop (no external command, so rubash's
//! external-finish sweep cannot fire), which makes the GNU side
//! deterministic (3/3 oracle runs) while staying red on base.

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

/// Defect cell: the coproc's output is consumed, an EOF read
/// proves it dead, and a builtin-only loop gives the notify
/// sweep its boundary — no external command runs, so only a
/// genuine command-boundary retirement (GNU's) can clear the
/// state. The third read must then fail on the empty expansion,
/// `b` stays unset, and both COPROC and COPROC_PID are gone.
/// Baseline rubash leaves the stale fd readable-as-EOF and the
/// variables set.
#[test]
fn dead_coproc_retired_at_builtin_only_boundary() {
    let (stdout, stderr, code) = run_script(
        "coproc { echo only; }\n\
         read -u ${COPROC[0]} a\n\
         read -u ${COPROC[0]} x\n\
         i=0; while (( i < 10000 )); do (( i++ )); done\n\
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

/// Regression lock: with a real external command boundary the
/// retirement already happens on base too (the external-finish
/// sweep); the fix must keep this working while extending the
/// retirement to builtin-only boundaries.
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

/// Regression lock: an EOF read plus an external boundary also
/// retires on base; state face of the same mechanism.
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
