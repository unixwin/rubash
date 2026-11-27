//! rubash#2344: traps on the stop class (TSTP/TTIN/TTOU) and CONT were
//! recorded in the trap table but never installed a kernel handler —
//! the managed-signal set behind `reconcile_kernel_trap_dispositions`
//! omitted exactly those four signals. A trapped CONT was silently
//! discarded; a trapped TSTP/TTIN/TTOU left the disposition at SIG_DFL,
//! so the shell really stopped and the pending trap was dropped on
//! resume; even `trap '' TSTP` never installed SIG_IGN.
//!
//! GNU installs the deferring handler for ANY trapped signal:
//! trap.c:838 set_signal -> sig.c:830 set_signal_handler (sa_flags=0),
//! with execution deferred through pending_traps[] to trap.c:328
//! run_pending_traps at a command boundary, so a trapped stop signal
//! never stops the shell. The stop class is NOT in GNU's terminating
//! set (sig.c:127 terminating_signals[]), so an armed EXIT trap must
//! not reroute an untrapped stop signal either — the kernel stop
//! stands. All expectations below are byte-verified against GNU bash
//! 5.3.20 via script files (audit ch1 RC1 + morphology m1a–m1h).
//!
//! Safety: a regression here STOPS the child shell, which would hang a
//! naive `.output()` forever. Every run below spawns the shell in its
//! own process group under a watchdog: after a grace period the
//! watchdog sends SIGCONT to the child's group (never the test's own
//! group), and a hard SIGKILL caps the run — a failing build fails the
//! assertion instead of hanging CI.
#![cfg(unix)]

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const GRACE: Duration = Duration::from_millis(2500);
const HARD: Duration = Duration::from_secs(12);

struct Outcome {
    stdout: String,
    stderr: String,
    code: Option<i32>,
    elapsed: Duration,
    /// The watchdog had to SIGCONT the child's group (it was still
    /// alive past the grace period — for a stopped child, the only way
    /// it ever finishes).
    watchdog_fired: bool,
    /// /proc showed the shell itself in State T (stopped) at some poll.
    stopped_seen: bool,
    /// /proc showed a direct child of the shell in State T.
    child_stopped_seen: bool,
}

#[cfg(target_os = "linux")]
fn proc_state(pid: u32) -> Option<String> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    status
        .lines()
        .find(|line| line.starts_with("State:"))
        .map(|line| line.to_string())
}

#[cfg(target_os = "linux")]
fn any_child_stopped(pid: u32) -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Ok(status) = std::fs::read_to_string(entry.path().join("status")) else {
            continue;
        };
        let is_child = status.lines().any(|line| line == format!("PPid:\t{pid}"));
        let stopped = status.lines().any(|line| line.starts_with("State:\tT"));
        if is_child && stopped {
            return true;
        }
    }
    false
}

/// Run `script` (written to a temp file, script-file form) under the
/// built rubash with the watchdog described in the module doc.
/// `ignore_tstp_on_entry` pre-ignores SIGTSTP across the exec (the
/// hard-ignore probe).
fn run_rubash(name: &str, script: &str, ignore_tstp_on_entry: bool) -> Outcome {
    use std::os::unix::process::CommandExt;
    let dir = std::env::temp_dir().join(format!("rubash-2344-{}-{}", name, std::process::id()));
    std::fs::create_dir_all(&dir).expect("create probe dir");
    let script_path = dir.join("case.sh");
    std::fs::write(&script_path, script).expect("write probe script");

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg(&script_path)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    if ignore_tstp_on_entry {
        // SAFETY: pre_exec runs in the forked child before exec; a
        // single signal() call is async-signal-safe there.
        unsafe {
            cmd.pre_exec(|| {
                libc::signal(libc::SIGTSTP, libc::SIG_IGN);
                Ok(())
            });
        }
    }
    let mut child = cmd.spawn().expect("spawn rubash");
    let pid = child.id();
    let mut out_pipe = child.stdout.take().expect("stdout pipe");
    let mut err_pipe = child.stderr.take().expect("stderr pipe");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out_pipe.read_to_end(&mut buf);
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err_pipe.read_to_end(&mut buf);
        buf
    });

    let start = Instant::now();
    let mut watchdog_fired = false;
    let mut stopped_seen = false;
    let mut child_stopped_seen = false;
    let code = loop {
        if let Some(status) = child.try_wait().expect("poll rubash") {
            break status.code();
        }
        let elapsed = start.elapsed();
        #[cfg(target_os = "linux")]
        {
            if proc_state(pid).is_some_and(|s| s.starts_with("State:\tT")) {
                stopped_seen = true;
            }
            if any_child_stopped(pid) {
                child_stopped_seen = true;
            }
        }
        if elapsed > GRACE && !watchdog_fired {
            watchdog_fired = true;
            // SIGCONT to the child's OWN group only (process_group(0)
            // made it a group leader); never our own group.
            unsafe { libc::kill(-(pid as i32), libc::SIGCONT) };
        }
        if elapsed > HARD {
            unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
            break child.wait().expect("reap rubash").code();
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let stdout = String::from_utf8_lossy(&out_thread.join().expect("stdout reader")).into_owned();
    let stderr = String::from_utf8_lossy(&err_thread.join().expect("stderr reader")).into_owned();
    let _ = std::fs::remove_dir_all(&dir);
    Outcome {
        stdout,
        stderr,
        code,
        elapsed: start.elapsed(),
        watchdog_fired,
        stopped_seen,
        child_stopped_seen,
    }
}

/// The issue seed, CONT half: a trapped CONT fires at the boundary.
#[test]
fn issue2344_cont_trap_fires() {
    let out = run_rubash(
        "cont",
        "trap 'echo CONT-H' CONT\nkill -CONT $$\necho after\n",
        false,
    );
    assert_eq!(out.stdout, "CONT-H\nafter\n", "stderr: {:?}", out.stderr);
    assert_eq!(out.code, Some(0));
    assert!(!out.watchdog_fired, "shell must finish on its own");
    assert!(!out.stopped_seen, "shell must never stop");
}

/// The issue seed, TSTP half: both traps fire in order, the shell
/// never stops (pre-fix it stopped for real and both traps were lost).
#[test]
fn issue2344_tstp_trap_fires_and_shell_never_stops() {
    let out = run_rubash(
        "tstp",
        "trap 'echo TSTP-H' TSTP\ntrap 'echo CONT-H' CONT\nkill -TSTP $$\necho after-tstp\nkill -CONT $$\necho after-cont\n",
        false,
    );
    assert_eq!(
        out.stdout, "TSTP-H\nafter-tstp\nCONT-H\nafter-cont\n",
        "stderr: {:?}",
        out.stderr
    );
    assert_eq!(out.code, Some(0));
    assert!(
        !out.watchdog_fired,
        "a trapped TSTP must not leave the shell stopped for the watchdog"
    );
    assert!(!out.stopped_seen, "shell must never enter State T");
}

/// TTIN and TTOU take the same handler path.
#[test]
fn issue2344_ttin_ttou_traps_fire() {
    let out = run_rubash(
        "ttin-ttou",
        "trap 'echo TTIN-H' TTIN\ntrap 'echo TTOU-H' TTOU\nkill -TTIN $$\necho mid\nkill -TTOU $$\necho end\n",
        false,
    );
    assert_eq!(
        out.stdout, "TTIN-H\nmid\nTTOU-H\nend\n",
        "stderr: {:?}",
        out.stderr
    );
    assert_eq!(out.code, Some(0));
    assert!(!out.watchdog_fired);
    assert!(!out.stopped_seen);
}

/// Deferral point (GNU jobs.c:3064 wait_for / trap.c:328): a TSTP
/// delivered while the shell waits for a foreground command dispatches
/// only AFTER the wait completes — the sleep runs its full course,
/// $? survives, and the trap prints before the next command's output.
#[test]
fn issue2344_trap_during_foreground_wait_defers_until_wait_completes() {
    let out = run_rubash(
        "defer",
        "trap 'echo TSTP-H' TSTP\n(sleep 0.3; kill -TSTP $$) &\nsleep 2\necho \"after rc=$?\"\n",
        false,
    );
    assert_eq!(
        out.stdout, "TSTP-H\nafter rc=0\n",
        "stderr: {:?}",
        out.stderr
    );
    assert_eq!(out.code, Some(0));
    assert!(
        out.elapsed >= Duration::from_millis(1500),
        "trap deferred past the full sleep (elapsed {:?})",
        out.elapsed
    );
    assert!(!out.stopped_seen, "shell must not stop mid-wait");
}

/// `trap '' TSTP` installs a real SIG_IGN (pre-fix the ignore was
/// recorded but never applied, and the shell stopped).
#[test]
fn issue2344_ignore_form_installs_sig_ign() {
    let out = run_rubash("ignore", "trap '' TSTP\nkill -TSTP $$\necho alive\n", false);
    assert_eq!(out.stdout, "alive\n", "stderr: {:?}", out.stderr);
    assert_eq!(out.code, Some(0));
    assert!(
        !out.watchdog_fired,
        "an ignored TSTP must not stop the shell for the watchdog"
    );
    assert!(!out.stopped_seen);
}

/// The other side of the invariant: an UNTRAPPED stop signal is not
/// rerouted by an armed EXIT trap (sig.c:127 terminating_signals[]
/// names no stop signal) — the kernel stop stands, and after SIGCONT
/// the script finishes normally and the EXIT trap runs at exit.
#[test]
fn issue2344_exit_trap_does_not_reroute_stop_signals() {
    let out = run_rubash(
        "exit-armed",
        "trap 'echo EXIT-H' EXIT\nkill -TSTP $$\necho alive\n",
        false,
    );
    assert_eq!(out.stdout, "alive\nEXIT-H\n", "stderr: {:?}", out.stderr);
    assert_eq!(out.code, Some(0));
    assert!(
        out.stopped_seen || out.watchdog_fired,
        "untrapped TSTP must still stop the shell (watchdog CONT proves the stop was real)"
    );
}

/// `trap - TSTP` restores the entry disposition: the stop is back.
#[test]
fn issue2344_reset_restores_kernel_stop() {
    let out = run_rubash(
        "reset",
        "trap 'echo TSTP-H' TSTP\ntrap - TSTP\nkill -TSTP $$\necho alive\n",
        false,
    );
    assert_eq!(out.stdout, "alive\n", "stderr: {:?}", out.stderr);
    assert_eq!(out.code, Some(0));
    assert!(
        out.stopped_seen || out.watchdog_fired,
        "after reset the shell must stop on TSTP again"
    );
}

/// A trapped TSTP interrupts the `wait` builtin with status 128+20
/// and the trap runs (GNU jobs.c wait_for + ADDINTERRUPT shape).
#[test]
fn issue2344_wait_builtin_interrupted_by_trapped_tstp() {
    let out = run_rubash(
        "wait",
        "trap 'echo TSTP-H' TSTP\nsleep 3 &\n(sleep 0.3; kill -TSTP $$) &\nwait\necho \"wait rc=$?\"\n",
        false,
    );
    assert_eq!(
        out.stdout, "TSTP-H\nwait rc=148\n",
        "stderr: {:?}",
        out.stderr
    );
    assert_eq!(out.code, Some(0));
    assert!(!out.stopped_seen);
}

/// Children do not inherit the handler: a child shell with no TSTP
/// trap of its own starts from the entry disposition (SIG_DFL) and
/// stops for real (GNU trap.c:1489 restore_original_signals), while
/// the parent's trap never fires for the child's delivery.
#[cfg(target_os = "linux")]
#[test]
fn issue2344_child_does_not_inherit_trap_handler() {
    let rubash = env!("CARGO_BIN_EXE_rubash");
    let script = format!(
        "trap 'echo PARENT-H' TSTP\n{rubash} -c 'kill -TSTP $$; echo child-after'\necho parent-after\n"
    );
    let out = run_rubash("child", &script, false);
    assert_eq!(
        out.stdout, "child-after\nparent-after\n",
        "stderr: {:?}",
        out.stderr
    );
    assert_eq!(out.code, Some(0));
    assert!(
        out.child_stopped_seen,
        "the untrapped child must stop on TSTP (it inherited SIG_DFL, not the handler)"
    );
}

/// A signal ignored on entry cannot be trapped (trap.c:850
/// SIG_HARD_IGNORE): the trap attempt is a no-op, the delivery is
/// swallowed, and the shell neither stops nor runs the action.
#[test]
fn issue2344_entry_ignored_tstp_cannot_be_trapped() {
    let out = run_rubash(
        "entry-ignored",
        "trap 'echo NOPE' TSTP\nkill -TSTP $$\necho alive\n",
        true,
    );
    assert_eq!(out.stdout, "alive\n", "stderr: {:?}", out.stderr);
    assert_eq!(out.code, Some(0));
    assert!(!out.watchdog_fired);
    assert!(!out.stopped_seen);
}
