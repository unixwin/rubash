//! rubash#2250 — PIPESTATUS wait race: a pipeline member whose write end
//! reached EOF (its downstream finished) but which is still alive — in
//! exit transit after its last write — was killed by the shell's wait
//! path and reported as a SIGPIPE death (141) instead of the status it
//! eventually exited with.
//!
//! GNU ground truth (bash 5.3.20): the pipeline wait is jobs.c:3064
//! `wait_for()` — it blocks for each member's own exit with no
//! wall-clock cap and never terminates a member from the wait path —
//! and jobs.c `process_exit_status()` surfaces the reaped status
//! verbatim (WIFSIGNALED -> 128+signal, else the exit code). A producer
//! that outlives its consumer dies of SIGPIPE only when it actually
//! writes again, delivered by the kernel; PIPESTATUS then records that
//! genuine death (see the `yes | head` guard pin below).
//!
//! The discriminator pins use a producer that closes fd 1 explicitly
//! and then lingers before exiting 7 (`exec 1>&-; sleep 1; exit 7`
//! under an external /bin/bash): downstream sees EOF immediately, the
//! member is unambiguously still alive, and GNU reports 7. The pre-fix
//! machinery reported 141 on both concurrent wait sites (the
//! single-try_wait reverse reap in the non-Windows concurrent executor,
//! and the 100 ms deadline arm of the shared member wait). The pins are
//! expected RED on the pre-fix build and GREEN after it. One guard pin
//! below (the late-write shape) is not a discriminator: it is 141 on
//! both builds by design, pinning GNU's consumer-lifetime semantics
//! against read-end-hoarding fixes.
//!
//! Unix-only: the shapes rely on explicit fd closing, `sleep` timing
//! and coreutils `yes`/`head` SIGPIPE semantics.
#![cfg(unix)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("rubash-2250-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch dir");
        Scratch(dir)
    }

    /// Script-file form: file in a fresh cwd, PATH/HOME pinned.
    fn rubash_file(&self, script: &str, args: &[&str]) -> (String, String, Option<i32>) {
        let file = self.0.join("case.sh");
        fs::write(&file, script).expect("write script");
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&file)
            .args(args)
            .current_dir(&self.0)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.0)
            .output()
            .expect("run rubash");
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
        )
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The producer writes one line, closes its stdout (the consumer sees
/// EOF and finishes), then lingers a full second before exiting 7.
/// GNU waits the member out: PIPESTATUS is `7 0`. The pre-fix reverse
/// reap killed the member at its single try_wait and recorded 141.
#[test]
fn issue2250_closed_stdout_lingering_producer_reports_real_status() {
    let scratch = Scratch::new("linger-external");
    let (out, err, rc) = scratch.rubash_file(
        "/bin/bash -c 'echo hi; exec 1>&-; sleep 1; exit 7' | cat\n\
         echo \"ps=${PIPESTATUS[0]} ${PIPESTATUS[1]}\"\n",
        &[],
    );
    assert_eq!(out, "hi\nps=7 0\n");
    assert_eq!(err, "");
    assert_eq!(rc, Some(0));
}

/// The streaming-tail discriminator: the producer closes fd 1 FIRST
/// (the tail side sees EOF instantly), then lingers a full second
/// before exiting 7, feeding an in-shell builtin tail (`true`). The
/// streaming-tail reap waits the producer against NO downstream, so
/// the pre-fix deadline armed immediately and the member was killed
/// 100 ms in — recorded 141. GNU reports the member's real 7.
///
/// The producer deliberately does NOT write before closing. With a
/// non-reading tail, a write races the consumer's exit: GNU runs the
/// tail as a forked child (execute_cmd.c:2620 execute_pipeline; the
/// parent closes its own read-end copy once the children are forked),
/// so the last read end closes when `true` exits — almost surely
/// before an exec'ing interpreter producer lands its first write —
/// and the producer dies of a GENUINE kernel SIGPIPE. Measured on
/// the GNU 5.3.20 oracle, the writing variant of this shape
/// (`echo hi; exec 1>&-; sleep 1; exit 7`) reports 141 in 15/15 runs,
/// so a writing producer cannot discriminate the wait race on this
/// route. The close-first shape can: no write exists that could
/// SIGPIPE, so any 141 is the wait path's own kill. Verified pre-fix
/// (master 050d3c87 build): this exact script reports a fast 141;
/// the fixed build and GNU report 7 after the full second.
#[test]
fn issue2250_streaming_tail_lingering_producer_reports_real_status() {
    let scratch = Scratch::new("linger-tail");
    let (out, err, rc) = scratch.rubash_file(
        "/bin/bash -c 'exec 1>&-; sleep 1; exit 7' | true\n\
         echo \"ps=${PIPESTATUS[0]} ${PIPESTATUS[1]}\"\n",
        &[],
    );
    assert_eq!(out, "ps=7 0\n");
    assert_eq!(err, "");
    assert_eq!(rc, Some(0));
}

/// Guard for the streaming tail's consumer-lifetime semantics (NOT a
/// wait-race discriminator — it reports 141 before and after the
/// fix): a producer that writes only AFTER the non-reading tail has
/// exited (0.3 s in; the tail returned after microseconds) writes
/// into a pipe with no readers and dies of the kernel's SIGPIPE —
/// GNU reports the genuine 141, and so must rubash. This pins the
/// boundary against "fixes" that would keep the pipe's read end open
/// until the producers are reaped: those would report 7 here and
/// hang `yes | true` outright, diverging from GNU's model where the
/// read end's lifetime is the consumer's own.
#[test]
fn issue2250_streaming_tail_late_write_dies_genuine_sigpipe() {
    let scratch = Scratch::new("linger-tail-late-write");
    let (out, err, rc) = scratch.rubash_file(
        "/bin/bash -c 'sleep 0.3; echo hi; exec 1>&-; sleep 1; exit 7' | true\n\
         echo \"ps=${PIPESTATUS[0]} ${PIPESTATUS[1]}\"\n",
        &[],
    );
    assert_eq!(out, "ps=141 0\n");
    assert_eq!(err, "");
    assert_eq!(rc, Some(0));
}

/// Guard: the genuine SIGPIPE face must survive the fix. `yes` keeps
/// writing after `head` exits, dies of the kernel's SIGPIPE, and its
/// reaped status is a genuine 141 — no synthesis involved either way.
#[test]
fn issue2250_genuine_sigpipe_death_still_reports_141() {
    let scratch = Scratch::new("genuine-sigpipe");
    let (out, err, rc) = scratch.rubash_file(
        "yes | head -3\n\
         echo \"ps=${PIPESTATUS[0]} ${PIPESTATUS[1]}\"\n",
        &[],
    );
    assert_eq!(out, "y\ny\ny\nps=141 0\n");
    assert_eq!(err, "");
    assert_eq!(rc, Some(0));
}

/// The original #2236-battery species, as a stability pin: a shell
/// child running a plain `echo hi` script piped into `cat` reported a
/// coin-flip 141 on the pre-fix build (the child is in exit transit
/// when the wait machinery looks). The member's real status is 0 on
/// every run after the fix.
#[test]
fn issue2250_shell_child_producer_reports_zero_stably() {
    let scratch = Scratch::new("child-producer");
    let inner = scratch.0.join("inner.sh");
    fs::write(&inner, "echo hi\n").expect("write inner script");
    let exe = env!("CARGO_BIN_EXE_rubash");
    let inner_arg = inner.to_string_lossy().into_owned();
    let (out, err, rc) = scratch.rubash_file(
        "for i in 1 2 3 4 5; do\n\
           \"$1\" \"$2\" | cat\n\
           echo \"ps=${PIPESTATUS[0]} ${PIPESTATUS[1]}\"\n\
         done\n",
        &[exe, &inner_arg],
    );
    assert_eq!(out, "hi\nps=0 0\n".repeat(5));
    assert_eq!(err, "");
    assert_eq!(rc, Some(0));
}
