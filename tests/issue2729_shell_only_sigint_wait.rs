//! rubash#2729 — a shell-only SIGINT received during a foreground
//! wait is re-raised after the child exits, whatever the child did.
//!
//! GNU ground truth (bash 5.3.20 oracle, probed with kill(2)
//! delivery to the shell process only): waitchld records
//! wait_sigint_received, then classifies the delivery by the
//! foreground job's outcome — jobs.c:4157-4158 sets
//! child_caught_sigint when the job exited normally OR died by a
//! signal other than SIGINT, jobs.c:4162-4163 clears it only when
//! the job died by SIGINT; wait_for's cleanup (jobs.c:4410ff) acts
//! on the pending SIGINT (trap / re-raise) only when
//! child_caught_sigint == 0. So: child exits normally -> the
//! SIGINT is discarded and the script continues; child dies by
//! SIGTERM -> discarded too (the Terminated notice prints, the
//! script continues at rc=143); child dies by SIGINT -> the shell
//! dies by SIGINT.
//!
//! rubash's wait_external_child (src/executor/external_inner.rs)
//! consumes the recorded shell-only SIGINT unconditionally at the
//! list boundary: the shell dies by SIGINT after the wait no
//! matter how the child ended.
//!
//! Harness: the shell runs a script file in its own process group
//! (process_group(0)); deliveries are kill(2) from the test
//! process, shell-pid-only or group-wide per cell. No PTY, no
//! job control — the script-shell geometry of the issue.

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
    let dir = std::env::temp_dir().join(format!("rubash-issue2729-{}-{}", std::process::id(), seq));
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
    // Latch: the script's first line creates the file once the
    // shell is executing; then give the foreground child a moment
    // to be the running wait target.
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

/// The shell's single foreground child (the running sleep).
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
        // comm may contain spaces/parens; ppid is the 2nd field
        // after the final ')'.
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

fn kill(pid: u32, sig: i32) {
    let rc = unsafe { libc::kill(pid as i32, sig) };
    assert_eq!(rc, 0, "kill({pid}, {sig}) failed");
}

fn kill_group(pid: u32, sig: i32) {
    let rc = unsafe { libc::kill(-(pid as i32), sig) };
    assert_eq!(rc, 0, "killpg({pid}, {sig}) failed");
}

const FG_SCRIPT: &str = ": > \"$J2_LATCH\"\n\
    echo STARTED\n\
    /usr/bin/sleep SLEEPSECS\n\
    echo \"AFTER rc=$?\"\n";

/// Cell A (defect): SIGINT to the shell only while it waits; the
/// child runs to normal completion. GNU discards the SIGINT: the
/// script continues and exits 0. Baseline rubash dies by SIGINT
/// after the wait and AFTER never prints.
#[test]
fn shell_only_sigint_discarded_when_child_exits_normally() {
    let rig = spawn_rig(&FG_SCRIPT.replace("SLEEPSECS", "2"));
    let t = Instant::now();
    kill(rig.pid, libc::SIGINT);
    let d = finish(rig, t);
    assert_eq!(
        d.status.code(),
        Some(0),
        "shell must survive its own discarded SIGINT: status {:?}\nstdout: {}\nstderr: {}",
        d.status,
        d.stdout,
        d.stderr
    );
    assert!(
        d.stdout.contains("AFTER rc=0"),
        "the list continues after the wait: {}",
        d.stdout
    );
    assert!(
        d.elapsed >= Duration::from_millis(1200),
        "the shell waited out the child: {:?}",
        d.elapsed
    );
}

/// Cell B (defect): SIGINT to the shell only, then the child is
/// killed by SIGTERM from outside. GNU discards the SIGINT (the
/// child did not die by SIGINT), prints the Terminated notice and
/// continues at rc=143.
#[test]
fn shell_only_sigint_discarded_when_child_dies_by_term() {
    let rig = spawn_rig(&FG_SCRIPT.replace("SLEEPSECS", "30"));
    let child = child_pid_of(rig.pid).expect("foreground child exists");
    let t = Instant::now();
    kill(rig.pid, libc::SIGINT);
    std::thread::sleep(Duration::from_millis(300));
    kill(child, libc::SIGTERM);
    let d = finish(rig, t);
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
        "the surviving shell prints the death notice: {}",
        d.stderr
    );
}

/// Cell C (control, regression lock): SIGINT to the whole group —
/// the child dies by SIGINT, and the shell dies by SIGINT too,
/// promptly. Both shells agree; this must keep working.
#[test]
fn group_sigint_kills_shell_by_sigint_control() {
    let rig = spawn_rig(&FG_SCRIPT.replace("SLEEPSECS", "30"));
    let t = Instant::now();
    kill_group(rig.pid, libc::SIGINT);
    let d = finish(rig, t);
    assert_eq!(
        d.status.signal(),
        Some(libc::SIGINT),
        "status {:?}\nstdout: {}\nstderr: {}",
        d.status,
        d.stdout,
        d.stderr
    );
    assert!(
        !d.stdout.contains("AFTER"),
        "no continuation after a group SIGINT: {}",
        d.stdout
    );
    assert!(
        d.elapsed < Duration::from_secs(5),
        "death is prompt: {:?}",
        d.elapsed
    );
}
