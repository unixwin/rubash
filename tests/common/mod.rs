//! Shared child-process harness helpers for the integration-test
//! binaries (rubash#2197).
//!
//! This module exists because integration-test crates cannot import
//! each other's helpers: the hardened `wait_bounded` landed for
//! rubash#2056 inside tests/regression.rs (PR #2187), while six
//! sibling history test binaries each kept a private copy of the
//! pre-hardening shape — poll `try_wait` until exit, then drain the
//! pipes with an unbounded `read_to_end`. Their waits did panic at
//! the bound, but the orphan shape slipped past it: the child exits,
//! a descendant keeps a pipe write end open, the post-exit read never
//! returns, and the nominal bound never fires (the shape that once
//! ate a full 25-minute goldens job in #1903). The copies are deleted
//! in favor of this one helper, so a future bound fix lands once.
//!
//! A host binary pulls the module in with `mod common;` at its top.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Outcome of a bounded child wait (mirrors the RunOutcome shape of
/// tests/regression.rs, rubash#2056).
pub struct BoundedOutcome {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub code: Option<i32>,
    pub timed_out: bool,
    /// Pid of the direct child: carried so a timeout failure can name
    /// the wedged process, not just the bound.
    pub pid: u32,
}

/// One child pipe being drained on its own thread (rubash#2056).
struct PipeDrain {
    buffer: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    done: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

fn start_drain<R: Read + Send + 'static>(pipe: Option<R>) -> PipeDrain {
    let drain = PipeDrain {
        buffer: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        done: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(pipe.is_none())),
    };
    if let Some(mut pipe) = pipe {
        let buffer = drain.buffer.clone();
        let done = drain.done.clone();
        std::thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            loop {
                match pipe.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => buffer.lock().unwrap().extend_from_slice(&chunk[..n]),
                }
            }
            done.store(true, std::sync::atomic::Ordering::SeqCst);
        });
    }
    drain
}

/// Bounded wait for a spawned child, hardened by rubash#2056 and
/// converged for the sibling history binaries by rubash#2197.
///
/// Both pipes drain on threads from entry, the wait polls
/// `try_wait`, and ONE deadline covers both the child's exit and the
/// pipes reaching EOF. A timeout kills the child and returns the
/// partial captures with the pid, so the failure the caller raises
/// carries its evidence. The timeout stays a returned outcome rather
/// than a panic here: the panic-with-diagnostics half of the shape
/// lives at the assertion layer of each caller.
pub fn wait_bounded(child: &mut Child, limit: Duration) -> BoundedOutcome {
    let pid = child.id();
    let stdout = start_drain(child.stdout.take());
    let stderr = start_drain(child.stderr.take());
    let snapshot = |timed_out: bool, code: Option<i32>| BoundedOutcome {
        stdout: stdout.buffer.lock().unwrap().clone(),
        stderr: stderr.buffer.lock().unwrap().clone(),
        code,
        timed_out,
        pid,
    };
    let start = Instant::now();
    let mut exit_code: Option<Option<i32>> = None;
    loop {
        if exit_code.is_none() {
            match child.try_wait() {
                Ok(Some(status)) => exit_code = Some(status.code()),
                Ok(None) => {}
                Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return snapshot(false, None);
                }
            }
        }
        if let Some(code) = exit_code {
            let drained = stdout.done.load(std::sync::atomic::Ordering::SeqCst)
                && stderr.done.load(std::sync::atomic::Ordering::SeqCst);
            if drained {
                return snapshot(false, code);
            }
        }
        if start.elapsed() > limit {
            if exit_code.is_none() {
                let _ = child.kill();
                let _ = child.wait();
            }
            return snapshot(true, exit_code.flatten());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// PATH for the fake-child pins (mirrors regression.rs's
/// minimal_path: System32 first on Windows, Git's usr/bin appended
/// when present so emulated coreutils resolve; /usr/bin:/bin else).
fn minimal_path() -> String {
    if cfg!(windows) {
        let mut path = r"C:\Windows\System32".to_string();
        let git_usr_bin = r"C:\Program Files\Git\usr\bin";
        if std::path::Path::new(git_usr_bin).is_dir() {
            path.push(';');
            path.push_str(git_usr_bin);
        }
        path
    } else {
        "/usr/bin:/bin".to_string()
    }
}

/// Fake child for the issue2197 wait_bounded pins: the rubash binary
/// itself running a `-c` script, pipes on both streams. What the
/// script does (hang, orphan its pipe) is the pin's variable; the
/// spawn shape is constant.
fn spawn_issue2197_fake_child(script: &str) -> Child {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(["-c", script])
        .env_clear()
        .env("PATH", minimal_path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd.spawn().expect("spawn issue2197 fake child")
}

/// rubash#2197, the #1903 orphan shape: the child EXITS promptly
/// with code 0 but leaves a background orphan holding the stdout
/// write end, so pipe EOF never arrives. The pre-convergence private
/// copies read the pipes only after exit with an unbounded
/// `read_to_end` — this shape hung them forever, past their nominal
/// bound. The shared helper must count the still-open pipe against
/// the same bound and return the partial capture with the child's
/// real exit code. This pin is compiled into every binary that
/// includes this module, so each converged binary proves it.
#[test]
fn issue2197_wait_bounded_bounds_post_exit_pipe_drain() {
    let mut child =
        spawn_issue2197_fake_child("echo issue2197-orphan-marker\nsleep 120 &\nexit 0\n");
    let started = Instant::now();
    let out = wait_bounded(&mut child, Duration::from_secs(3));
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(30),
        "the 3s bound did not fire promptly: {elapsed:?}"
    );
    assert!(
        out.timed_out,
        "pipe held open by an orphan past the bound must report timed_out; got code {:?}",
        out.code
    );
    assert_eq!(out.code, Some(0), "the child itself exited 0");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("issue2197-orphan-marker"),
        "partial stdout lost: {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// rubash#2197: a child that prints markers and then never exits
/// must be killed at the bound, and the timeout outcome must carry
/// whatever it printed before wedging — the pre-convergence copies
/// panicked on this shape with empty captures, so a hang surfaced
/// with zero evidence.
#[test]
fn issue2197_wait_bounded_kills_never_exiting_child_with_partial_output() {
    let mut child = spawn_issue2197_fake_child(
        "echo issue2197-out-marker\necho issue2197-err-marker >&2\nsleep 120\n",
    );
    let pid = child.id();
    let started = Instant::now();
    let out = wait_bounded(&mut child, Duration::from_secs(3));
    let elapsed = started.elapsed();
    assert!(
        out.timed_out,
        "never-exiting child must report timed_out; got code {:?}",
        out.code
    );
    assert_eq!(out.pid, pid, "outcome must carry the child's pid");
    assert!(
        elapsed < Duration::from_secs(30),
        "the 3s bound did not fire promptly: {elapsed:?}"
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("issue2197-out-marker"),
        "partial stdout lost on timeout: {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("issue2197-err-marker"),
        "partial stderr lost on timeout: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        matches!(child.try_wait(), Ok(Some(_))),
        "timed-out child was not killed and reaped"
    );
}
