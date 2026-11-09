//! rubash#2730 — the emulated `sleep` has no real child for the
//! wait machinery.
//!
//! GNU ground truth (bash 5.3.20 oracle, probed with kill(2)
//! delivery; GNU has no sleep builtin, so its `sleep` is a real
//! child in the shell's process group, waited on by jobs.c:3064
//! wait_for):
//!   - group SIGINT: the child dies at once; the shell dies by
//!     SIGINT immediately (the child died by SIGINT, so the
//!     pending SIGINT is acted on — jobs.c:4410ff), the EXIT trap
//!     runs, the rest of the script never prints.
//!   - shell-only SIGTERM: the terminating-signal handler
//!     (sig.c:506 termsig_sighandler, installed with sa_flags=0 at
//!     sig.c:246-247) records the signal and the reap loop's
//!     CHECK_TERMSIG (jobs.c:4135) kills the shell at once —
//!     measured elapsed ~0.00 against a 10s child, which is left
//!     orphaned. NOTE: the issue text says GNU "defers" this
//!     delivery; that does not reproduce in the script-shell
//!     geometry (it appears to describe the niu interactive
//!     geometry the audit measured through). These pins follow
//!     the oracle, not the issue text; see the lane REPORT.
//!   - both control faces (real /usr/bin/sleep) behave the same.
//!
//! rubash face 1 (per the issue): the emulated sleep
//! (src/executor/command_dispatch_primary.rs PrimarySlot::Sleep,
//! trap_exec.rs sleep_interruptibly) cannot be group-killed — a
//! group SIGINT is recorded and consumed only when the virtual
//! sleep ends, so the script CONTINUES (END prints) and the shell
//! dies by SIGINT at process exit instead of at once.
//!
//! Harness: same script-shell geometry as the #2729 pins.

#![cfg(all(unix, target_os = "linux"))]

use std::io::Write;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

struct Driven {
    status: ExitStatus,
    stdout: String,
    stderr: String,
    elapsed: Duration,
}

struct Rig {
    child: Child,
    dir: PathBuf,
    pid: u32,
}

fn spawn_rig(body: &str) -> Rig {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rubash-issue2730-{}-{}", std::process::id(), seq));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let script = dir.join("s.sh");
    let mut f = std::fs::File::create(&script).expect("write script");
    f.write_all(body.as_bytes()).expect("write body");
    drop(f);
    let latch = dir.join("latch");
    let stdout = std::fs::File::create(dir.join("out")).expect("stdout file");
    let stderr = std::fs::File::create(dir.join("err")).expect("stderr file");
    let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script)
        .env("J2_LATCH", &latch)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .process_group(0)
        .spawn()
        .expect("spawn rubash script shell");
    let pid = child.id();
    let start = Instant::now();
    while !latch.exists() {
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "script never latched"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    std::thread::sleep(Duration::from_millis(400));
    Rig { child, dir, pid }
}

fn finish(mut rig: Rig, delivered_at: Instant) -> Driven {
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        match rig.child.try_wait().expect("try_wait") {
            Some(status) => break status,
            None => {
                if Instant::now() > deadline {
                    unsafe {
                        libc::kill(-(rig.pid as i32), libc::SIGKILL);
                    }
                    let _ = rig.child.kill();
                    panic!("shell did not settle within 20s of delivery");
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    };
    let elapsed = delivered_at.elapsed();
    let stdout = std::fs::read_to_string(rig.dir.join("out")).unwrap_or_default();
    let stderr = std::fs::read_to_string(rig.dir.join("err")).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&rig.dir);
    Driven {
        status,
        stdout,
        stderr,
        elapsed,
    }
}

const EMULATED: &str = "trap 'echo EXIT-TRAP-RAN' EXIT\n\
    : > \"$J2_LATCH\"\n\
    echo STARTED\n\
    sleep 2\n\
    echo END\n";

const REAL: &str = "trap 'echo EXIT-TRAP-RAN' EXIT\n\
    : > \"$J2_LATCH\"\n\
    echo STARTED\n\
    /usr/bin/sleep 2\n\
    echo END\n";

fn assert_prompt_signal_death(d: &Driven, sig: i32) {
    assert_eq!(
        d.status.signal(),
        Some(sig),
        "status {:?}\nstdout: {}\nstderr: {}",
        d.status,
        d.stdout,
        d.stderr
    );
    assert!(
        d.elapsed < Duration::from_millis(1500),
        "death must be prompt, not at the sleep's natural end: {:?}",
        d.elapsed
    );
    assert!(
        d.stdout.contains("EXIT-TRAP-RAN"),
        "the EXIT trap runs on the signal death: {}",
        d.stdout
    );
    assert!(
        !d.stdout.contains("END"),
        "the script must not continue past the killed sleep: {}",
        d.stdout
    );
}

/// Defect cell, face 1: group SIGINT during the EMULATED sleep.
/// GNU's real child dies at once and the shell dies by SIGINT at
/// once. Baseline rubash sleeps on, prints END, and dies by
/// SIGINT only at process exit.
#[test]
fn group_sigint_during_emulated_sleep_kills_promptly() {
    let rig = spawn_rig(EMULATED);
    let t = Instant::now();
    let rc = unsafe { libc::kill(-(rig.pid as i32), libc::SIGINT) };
    assert_eq!(rc, 0, "killpg(SIGINT) failed");
    let d = finish(rig, t);
    assert_prompt_signal_death(&d, libc::SIGINT);
}

/// Face 2 cell: shell-only SIGTERM during the EMULATED sleep.
/// GNU (real child, oracle-probed) dies by SIGTERM at once, EXIT
/// trap runs, END never prints. Whether baseline rubash diverges
/// here is what this cell measures; the fix must land on the
/// oracle shape either way.
#[test]
fn shell_only_sigterm_during_emulated_sleep_kills_promptly() {
    let rig = spawn_rig(EMULATED);
    let t = Instant::now();
    let rc = unsafe { libc::kill(rig.pid as i32, libc::SIGTERM) };
    assert_eq!(rc, 0, "kill(SIGTERM) failed");
    let d = finish(rig, t);
    assert_prompt_signal_death(&d, libc::SIGTERM);
}

/// Control (regression lock): shell-only SIGTERM during a REAL
/// sleep — the shell dies by SIGTERM at once in both shells.
#[test]
fn shell_only_sigterm_during_real_sleep_control() {
    let rig = spawn_rig(REAL);
    let t = Instant::now();
    let rc = unsafe { libc::kill(rig.pid as i32, libc::SIGTERM) };
    assert_eq!(rc, 0, "kill(SIGTERM) failed");
    let d = finish(rig, t);
    assert_prompt_signal_death(&d, libc::SIGTERM);
}

/// Control (regression lock): group SIGINT during a REAL sleep —
/// prompt death by SIGINT in both shells.
#[test]
fn group_sigint_during_real_sleep_control() {
    let rig = spawn_rig(REAL);
    let t = Instant::now();
    let rc = unsafe { libc::kill(-(rig.pid as i32), libc::SIGINT) };
    assert_eq!(rc, 0, "killpg(SIGINT) failed");
    let d = finish(rig, t);
    assert_prompt_signal_death(&d, libc::SIGINT);
}
