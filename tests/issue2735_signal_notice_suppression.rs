//! rubash#2735 — signal-death notices are printed even when the
//! shell dies by the same signal.
//!
//! GNU ground truth (bash 5.3.20 oracle, probed with kill(2)
//! group delivery): the job-death notice is printed by
//! notify_of_job_status (jobs.c:4626ff foreground pretty-print),
//! reached from wait_for's notify_and_cleanup (jobs.c:3385) only
//! when wait_for completes. A shell that itself receives the
//! terminating signal while reaping never gets there: the reap
//! loop breaks on terminating_signal and CHECK_TERMSIG
//! (quit.h:53; call site jobs.c:4135) runs termsig_handler
//! (sig.c:590), which exits the shell by that signal first. So a
//! group SIGTERM/SIGHUP that kills both child and shell leaves
//! stderr EMPTY; the notice appears only when the shell survives
//! (child-only delivery).
//!
//! rubash's foreground-reap path (note_child_signal_death /
//! report_pending_signal_notices) prints the notice regardless of
//! the shell's own pending death by the same signal.
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
}

struct Rig {
    child: Child,
    dir: PathBuf,
    pid: u32,
}

fn spawn_rig(body: &str) -> Rig {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rubash-issue2735-{}-{}", std::process::id(), seq));
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

fn child_pid_of(shell_pid: u32) -> Option<u32> {
    for entry in std::fs::read_dir("/proc").ok()? {
        let entry = entry.ok()?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        let Some((_, rest)) = stat.rsplit_once(')') else {
            continue;
        };
        let fields: Vec<&str> = rest.split_whitespace().collect();
        if fields.len() > 1 && fields[1].parse::<u32>() == Ok(shell_pid) {
            return name.parse().ok();
        }
    }
    None
}

fn finish(mut rig: Rig) -> Driven {
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
    let stdout = std::fs::read_to_string(rig.dir.join("out")).unwrap_or_default();
    let stderr = std::fs::read_to_string(rig.dir.join("err")).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&rig.dir);
    Driven {
        status,
        stdout,
        stderr,
    }
}

const FG_SCRIPT: &str = ": > \"$J2_LATCH\"\n\
    echo STARTED\n\
    /usr/bin/sleep 30\n\
    echo \"AFTER rc=$?\"\n";

/// Defect cell: group SIGTERM kills child and shell together.
/// The shell dies by SIGTERM and must NOT print the Terminated
/// notice for the job it died alongside.
#[test]
fn group_term_shell_dies_silently() {
    let rig = spawn_rig(FG_SCRIPT);
    let pid = rig.pid;
    let rc = unsafe { libc::kill(-(pid as i32), libc::SIGTERM) };
    assert_eq!(rc, 0, "killpg(SIGTERM) failed");
    let d = finish(rig);
    assert_eq!(
        d.status.signal(),
        Some(libc::SIGTERM),
        "status {:?}\nstdout: {}\nstderr: {}",
        d.status,
        d.stdout,
        d.stderr
    );
    assert!(
        !d.stderr.contains("Terminated"),
        "no notice when the shell dies by the same signal: {:?}",
        d.stderr
    );
    assert!(!d.stdout.contains("AFTER"), "no continuation: {}", d.stdout);
}

/// Defect cell: same shape with SIGHUP / Hangup.
#[test]
fn group_hup_shell_dies_silently() {
    let rig = spawn_rig(FG_SCRIPT);
    let pid = rig.pid;
    let rc = unsafe { libc::kill(-(pid as i32), libc::SIGHUP) };
    assert_eq!(rc, 0, "killpg(SIGHUP) failed");
    let d = finish(rig);
    assert_eq!(
        d.status.signal(),
        Some(libc::SIGHUP),
        "status {:?}\nstdout: {}\nstderr: {}",
        d.status,
        d.stdout,
        d.stderr
    );
    assert!(
        !d.stderr.contains("Hangup"),
        "no notice when the shell dies by the same signal: {:?}",
        d.stderr
    );
}

/// Control (regression lock): SIGTERM to the child only — the
/// shell survives, prints the Terminated notice and continues at
/// rc=143. Both shells agree; this must keep working.
#[test]
fn child_only_term_notice_printed_control() {
    let rig = spawn_rig(FG_SCRIPT);
    let child = child_pid_of(rig.pid).expect("foreground child exists");
    let rc = unsafe { libc::kill(child as i32, libc::SIGTERM) };
    assert_eq!(rc, 0, "kill(child, SIGTERM) failed");
    let d = finish(rig);
    assert_eq!(
        d.status.code(),
        Some(0),
        "status {:?}\nstdout: {}\nstderr: {}",
        d.status,
        d.stdout,
        d.stderr
    );
    assert!(
        d.stdout.contains("AFTER rc=143"),
        "the list continues at the child's signal status: {}",
        d.stdout
    );
    assert!(
        d.stderr.contains("Terminated"),
        "the surviving shell prints the notice: {:?}",
        d.stderr
    );
}
