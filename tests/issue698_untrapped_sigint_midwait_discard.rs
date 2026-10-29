//! niubash#698 — an untrapped Ctrl+C delivered through the engine
//! mailbox while an interactive session is blocked in a foreground
//! wait must settle the command at 130 and return control to the
//! driver (GNU: jobs.c:3064 wait_for observes the recorded ^C;
//! jobs.c:3315 ADDINTERRUPT throws to the top-level reader as a
//! DISCARD), not unwind the unit as an exit jump. The mid-wait
//! mailbox drain (`observe_signals_while_blocked`) converted an
//! untrapped delivery to a bare `ExitCode(128+sig)` without the
//! discard classification its two sibling drains make (the boundary
//! dispatch in `run_pending_signal_traps`; the `wait` builtin's
//! drain, rubash#1825), so the source driver recorded an exit jump,
//! the Session reported `Flow::ExitRequested(130)`, and niu's REPL —
//! which honors the discard mark when one is set (niubash#610) —
//! ended the whole process at rc=130 on the first untrapped Ctrl+C
//! of a foreground `sleep 10`. Verified on a real Windows console by
//! the niubash#698 native-harness re-verification.
//!
//! The delivery queue is process-global, so every test in this
//! binary serializes on one mutex (the session_api_interrupt.rs
//! discipline), and each in-process scenario runs its session on a
//! runner thread while a peer session performs the Tier-A
//! `inject_signal` — the same queue niu's console path feeds via
//! `inject_console_ctrl_c`.
//!
//! Windows carries the discriminating faces (the mailbox wait paths
//! are the Windows port of GNU's handler-installed wait; on unix the
//! kernel adjudicates foreground waits and the queued-delivery
//! rules of §6.4 apply instead). The unix section pins the
//! surrounding invariants: the interactive PTY face stays GNU, the
//! non-interactive die-by-signal shape is preserved, and a queued
//! kill(2)-class SIGINT consumed mid-sleep is forgotten with its
//! classification (it must not leak a discard mark into a later
//! genuine `exit` of the same unit).

use std::sync::Mutex;
use std::time::{Duration, Instant};

static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Ceiling for an interrupted run: the natural end of the probed
/// commands is 30s, so settling past this means the interrupt was
/// slept through.
#[cfg(windows)]
const INTERRUPT_LIMIT: Duration = Duration::from_secs(20);

#[cfg(windows)]
mod windows_faces {
    use super::*;
    use rubash::api::{Flow, RunOptions, Session};
    use std::path::PathBuf;

    struct DrivenRun {
        status: i32,
        flow: Flow,
        elapsed: Duration,
        after_status: i32,
    }

    /// Run `script` on an interactive-flagged session in a runner
    /// thread; once the handshake file exists (and the run has had a
    /// moment to block in its wait), queue an untrapped SIGINT from
    /// a peer session and collect the run's outcome plus a follow-up
    /// run's status.
    fn drive_interactive(script_body: &str, tag: &str) -> DrivenRun {
        let dir =
            std::env::temp_dir().join(format!("rubash-issue698-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create issue698 scratch dir");
        let started: PathBuf = dir.join("started.txt");
        let started_text = started.display().to_string();
        let script = format!(": > \"{started_text}\"; {script_body}");

        let runner = std::thread::spawn(move || {
            let mut session = Session::builder()
                .env("__RUBASH_INTERACTIVE", "1")
                .build()
                .expect("interactive session builds");
            let mut options = RunOptions::default();
            options.interactive = true;
            let t0 = Instant::now();
            let outcome = session
                .run(&script, &options)
                .expect("the interrupted run is an outcome, not an error");
            let elapsed = t0.elapsed();
            let after = session
                .run("echo alive", &RunOptions::default())
                .expect("the session is usable after the interrupt");
            DrivenRun {
                status: outcome.status,
                flow: outcome.flow,
                elapsed,
                after_status: after.status,
            }
        });

        let mut waited = Duration::ZERO;
        while !started.exists() {
            assert!(
                waited < Duration::from_secs(30),
                "the run never reached its foreground wait"
            );
            std::thread::sleep(Duration::from_millis(25));
            waited += Duration::from_millis(25);
        }
        std::thread::sleep(Duration::from_millis(400));
        let mut peer = Session::builder().build().expect("peer session builds");
        peer.inject_signal(2);

        // Join with a bound well past the interrupt limit so a
        // slept-through interrupt fails as a timeout, not a hang.
        let deadline = Instant::now() + INTERRUPT_LIMIT + Duration::from_secs(15);
        loop {
            if runner.is_finished() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the interrupted run never settled"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        runner.join().expect("runner thread")
    }

    /// The niu #698 composition itself: interactive session, builtin
    /// `sleep` in the foreground, mailbox Ctrl+C. Pre-fix the run
    /// reported Flow::ExitRequested(130) (which niu's REPL turns
    /// into process death); the fixed classification completes the
    /// run at 130 and the session lives on.
    #[test]
    fn issue698_interactive_builtin_sleep_mailbox_ctrl_c_discards() {
        let _guard = serial();
        let run = drive_interactive("sleep 30", "builtin-sleep");
        assert!(
            run.elapsed < INTERRUPT_LIMIT,
            "the interrupt was slept through: {:?}",
            run.elapsed
        );
        assert_eq!(
            run.status, 130,
            "the interrupted sleep settles at 128+SIGINT"
        );
        assert!(
            matches!(run.flow, Flow::Completed),
            "an untrapped Ctrl+C in an interactive session is a DISCARD, not an exit request: {:?}",
            run.flow
        );
        assert_eq!(
            run.after_status, 0,
            "the session continues after the discard"
        );
    }

    /// The external-command face of the same wait: `ping` is the
    /// #2020 probe vehicle (a pure external foreground command whose
    /// wait slices on the mailbox).
    #[test]
    fn issue698_interactive_external_mailbox_ctrl_c_discards() {
        let _guard = serial();
        let run = drive_interactive("ping -n 30 127.0.0.1", "external-ping");
        assert!(
            run.elapsed < INTERRUPT_LIMIT,
            "the interrupt was slept through: {:?}",
            run.elapsed
        );
        assert_eq!(run.status, 130);
        assert!(
            matches!(run.flow, Flow::Completed),
            "the external wait's untrapped Ctrl+C must classify DISCARD too: {:?}",
            run.flow
        );
        assert_eq!(run.after_status, 0);
    }

    /// The classification is interactive-only: a non-interactive
    /// session's untrapped mailbox SIGINT keeps the exit-jump
    /// classification byte-identically (the §6.4 / script-death
    /// shape), it just settles promptly instead of at natural end.
    #[test]
    fn issue698_noninteractive_mailbox_ctrl_c_still_exit_jumps() {
        let _guard = serial();
        let dir = std::env::temp_dir().join(format!(
            "rubash-issue698-{}-noninteractive",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create issue698 scratch dir");
        let started = dir.join("started.txt");
        let started_text = started.display().to_string();

        let runner = std::thread::spawn(move || {
            let mut session = Session::builder().build().expect("session builds");
            let t0 = Instant::now();
            let outcome = session
                .run(
                    &format!(": > \"{started_text}\"; sleep 30"),
                    &RunOptions::default(),
                )
                .expect("the interrupted run is an outcome, not an error");
            (outcome.status, outcome.flow, t0.elapsed())
        });
        let mut waited = Duration::ZERO;
        while !started.exists() {
            assert!(waited < Duration::from_secs(30), "the run never started");
            std::thread::sleep(Duration::from_millis(25));
            waited += Duration::from_millis(25);
        }
        std::thread::sleep(Duration::from_millis(400));
        let mut peer = Session::builder().build().expect("peer session builds");
        peer.inject_signal(2);
        let deadline = Instant::now() + INTERRUPT_LIMIT + Duration::from_secs(15);
        loop {
            if runner.is_finished() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the interrupted run never settled"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        let (status, flow, elapsed) = runner.join().expect("runner thread");
        assert!(elapsed < INTERRUPT_LIMIT, "slept through: {elapsed:?}");
        assert_eq!(status, 130);
        assert!(
            matches!(flow, Flow::ExitRequested(130)),
            "non-interactive untrapped SIGINT keeps the exit classification: {flow:?}"
        );
    }
}

#[cfg(unix)]
mod unix_faces {
    use super::*;
    use rubash::api::{Flow, RunOptions, Session};
    use std::io::{Read, Write};
    use std::os::unix::process::ExitStatusExt;
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc;

    // ---- PTY interactive face (the script(1) discipline of the
    // #1825 pins): the standalone binary's kernel-delivered ^C
    // during a foreground external command already classifies
    // DISCARD on unix; pin that it stays GNU-shaped.

    struct PtySession {
        child: Child,
        stdin: std::process::ChildStdin,
        rx: mpsc::Receiver<Vec<u8>>,
        transcript: String,
    }

    fn spawn_pty_session() -> PtySession {
        let cmdline = format!("{} -i", env!("CARGO_BIN_EXE_rubash"));
        let mut child = Command::new("script")
            .args(["-qec", &cmdline, "/dev/null"])
            .env("TERM", "xterm")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn script(1)");
        let stdin = child.stdin.take().expect("child stdin");
        let mut stdout = child.stdout.take().expect("child stdout");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match stdout.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        let mut session = PtySession {
            child,
            stdin,
            rx,
            transcript: String::new(),
        };
        session.send_line("echo READY_$((30+12))");
        session.wait_for("READY_42", Duration::from_secs(30));
        session.transcript.clear();
        session
    }

    impl PtySession {
        fn pump(&mut self, dur: Duration) {
            let deadline = Instant::now() + dur;
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return;
                }
                match self.rx.recv_timeout(left) {
                    Ok(chunk) => {
                        self.transcript.push_str(&String::from_utf8_lossy(&chunk));
                    }
                    Err(_) => return,
                }
            }
        }

        fn wait_for(&mut self, marker: &str, timeout: Duration) {
            let deadline = Instant::now() + timeout;
            while !self.transcript.contains(marker) {
                let left = deadline.saturating_duration_since(Instant::now());
                assert!(
                    !left.is_zero(),
                    "timed out waiting for {marker:?}; transcript: {:?}",
                    self.transcript
                );
                match self.rx.recv_timeout(left) {
                    Ok(chunk) => {
                        self.transcript.push_str(&String::from_utf8_lossy(&chunk));
                    }
                    Err(_) => break,
                }
            }
            assert!(
                self.transcript.contains(marker),
                "marker {marker:?} never appeared; transcript: {:?}",
                self.transcript
            );
        }

        fn send_line(&mut self, line: &str) {
            let _ = self.stdin.write_all(line.as_bytes());
            let _ = self.stdin.write_all(b"\n");
            let _ = self.stdin.flush();
        }

        fn send_ctrl_c(&mut self) {
            let _ = self.stdin.write_all(b"\x03");
            let _ = self.stdin.flush();
        }

        fn finish(mut self) {
            self.send_line("exit");
            self.pump(Duration::from_secs(2));
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn script_available() -> bool {
        Command::new("script").arg("--version").output().is_ok()
    }

    #[test]
    fn issue698_interactive_pty_foreground_sleep_ctrl_c_discards() {
        let _guard = serial();
        if !script_available() {
            return;
        }
        let mut session = spawn_pty_session();
        session.send_line("sleep 10");
        session.pump(Duration::from_millis(1200));
        session.send_ctrl_c();
        session.pump(Duration::from_millis(1000));
        session.send_line("echo NEXT_$?");
        // If the session died at the ^C, script(1) exits with it and
        // NEXT_130 never appears.
        session.wait_for("NEXT_130", Duration::from_secs(15));
        session.send_line("echo STILL_$((6*7))");
        session.wait_for("STILL_42", Duration::from_secs(10));
        session.finish();
    }

    /// The non-interactive shape the fix must preserve byte-for-byte:
    /// a `-c` shell whose foreground command is interrupted by a
    /// group SIGINT dies BY the signal, immediately (GNU parity,
    /// probed against GNU bash 5.3.20).
    #[test]
    fn issue698_noninteractive_group_sigint_dies_by_signal() {
        let _guard = serial();
        use std::os::unix::process::CommandExt;
        let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .args(["-c", "sleep 10"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .expect("spawn non-interactive rubash");
        std::thread::sleep(Duration::from_millis(700));
        let t0 = Instant::now();
        let pgid = child.id() as i32;
        let rc = unsafe { libc::kill(-pgid, libc::SIGINT) };
        assert_eq!(rc, 0, "killpg(SIGINT) failed");
        let status = child.wait().expect("wait for the shell");
        assert!(
            t0.elapsed() < Duration::from_secs(5),
            "the shell did not die promptly: {:?}",
            t0.elapsed()
        );
        assert_eq!(
            status.signal(),
            Some(libc::SIGINT),
            "a non-interactive shell dies by SIGINT itself, not by exit 130: {status}"
        );
    }

    // ---- The mid-sleep fold bookkeeping (unix): a queued
    // kill(2)-class SIGINT that lands during an interactive
    // session's builtin sleep is consumed by the mid-wait drain and
    // then FORGOTTEN (GNU: the shell-only delivery leaves the real
    // child running; the virtual child sleeps on, rubash#1640). Its
    // discard classification must be forgotten with it — a mark left
    // pending would misclassify a later genuine `exit` in the same
    // unit as a discard and the session would refuse to exit.

    struct DrivenRun {
        status: i32,
        flow: Flow,
        elapsed: Duration,
    }

    fn drive_interactive_unit(unit_body: &str, tag: &str) -> DrivenRun {
        let dir =
            std::env::temp_dir().join(format!("rubash-issue698-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create issue698 scratch dir");
        let started: PathBuf = dir.join("started.txt");
        let started_text = started.display().to_string();
        let unit = format!(": > \"{started_text}\"; {unit_body}");

        let runner = std::thread::spawn(move || {
            let mut session = Session::builder()
                .env("__RUBASH_INTERACTIVE", "1")
                .build()
                .expect("interactive session builds");
            let mut options = RunOptions::default();
            options.interactive = true;
            let t0 = Instant::now();
            let outcome = session
                .run(&unit, &options)
                .expect("the run is an outcome, not an error");
            DrivenRun {
                status: outcome.status,
                flow: outcome.flow,
                elapsed: t0.elapsed(),
            }
        });
        let mut waited = Duration::ZERO;
        while !started.exists() {
            assert!(waited < Duration::from_secs(30), "the unit never started");
            std::thread::sleep(Duration::from_millis(25));
            waited += Duration::from_millis(25);
        }
        std::thread::sleep(Duration::from_millis(400));
        let mut peer = Session::builder().build().expect("peer session builds");
        peer.inject_signal(2);
        runner.join().expect("runner thread")
    }

    #[test]
    fn issue698_forgotten_mid_sleep_sigint_leaves_no_discard_mark() {
        let _guard = serial();
        // The queued SIGINT lands mid-sleep and is forgotten there;
        // the unit's real `exit 3` must still classify as an exit.
        let run = drive_interactive_unit("sleep 3; exit 3", "fold-bookkeeping");
        assert!(
            run.elapsed >= Duration::from_millis(2500),
            "the shell-only delivery must not abort the sleep: {:?}",
            run.elapsed
        );
        assert_eq!(run.status, 3);
        assert!(
            matches!(run.flow, Flow::ExitRequested(3)),
            "a genuine exit after a forgotten interrupt stays an exit: {:?}",
            run.flow
        );
    }

    #[test]
    fn issue698_forgotten_mid_sleep_sigint_completes_normally() {
        let _guard = serial();
        let run = drive_interactive_unit("sleep 3", "fold-normal");
        assert!(
            run.elapsed >= Duration::from_millis(2500),
            "the sleep must run to completion: {:?}",
            run.elapsed
        );
        assert_eq!(run.status, 0);
        assert!(
            matches!(run.flow, Flow::Completed),
            "a forgotten delivery changes nothing: {:?}",
            run.flow
        );
    }
}
