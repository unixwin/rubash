//! rubash#3467 — [windows] Foreground Ctrl+Z needs an engine stop
//! surface: `inject_console_ctrl_z` + Windows Stopped arm (companion
//! to niubash#877).
//!
//! Before this fix the engine had exactly one static console-signal
//! injection face (`inject_console_ctrl_c`, signal 2) and no stop
//! face at all: an untrapped TSTP (20) observed mid-wait took the
//! same path as any untrapped terminating delivery — the foreground
//! wait ended at 128+20 and, for a real child, killed it. `jobs`
//! showed nothing and `fg` had nothing to resume. The fix adds the
//! static face (`script_driver::inject_console_ctrl_z`, mirroring
//! `inject_console_ctrl_c`) and a Windows Stopped arm in the two
//! non-unix foreground waits (`wait_external_child`, and the virtual
//! `sleep_interruptibly`).
//!
//! Stopped carrier for the virtual sleep: the in-process emulation
//! has no pid and no child handle, so the stop materializes one —
//! this binary is spawned as a real sleeper child (`rubash -c
//! "sleep <remaining>"`), suspended at once, and parked as the
//! stopped job. fg/bg/wait/kill then work through the ordinary
//! real-child machinery. The spawn is lazy (only on ^Z).
//!
//! GNU oracle (bash 5.2 in a pty, verified while writing this fix):
//! `sleep 5; echo CONTINUED` + ^Z prints `[1]+  Stopped  sleep 5`
//! and STILL runs CONTINUED — the list continues after a stop at
//! top level (the stop only abandons the rest inside loops, via the
//! breaking count). The Windows arm therefore returns the plain
//! 128+20 status like the unix stopped wait, with no discard mark
//! and no death record: a stop is neither a death nor a discard.
//!
//! Driven like the #698 pin: the delivery queue is process-global,
//! so every test serializes on one mutex, each scenario runs its
//! session on a runner thread, and the TSTP is injected from the
//! main thread — through the new static face where the face itself
//! is under test, through Tier-A `inject_signal` where the wait arms
//! are. On Windows `sleep` takes the in-process fast path only when
//! it resolves in PATH, so the tests plant a zero-byte `sleep.exe`
//! in a scratch dir on the session PATH (the dummy is never
//! executed: the fast path answers in-process).
//!
//! The unix section pins the surrounding invariants: the unix sleep
//! is a real child since #3488 and its behavior is untouched, and a
//! self-queued TSTP on unix keeps its pre-existing mailbox verdict
//! (the kernel owns stops there; only a real terminal ^Z stops a
//! unix job, via waitpid).

use std::sync::Mutex;
use std::time::{Duration, Instant};

static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Ceiling for a stopped run: the probed sleeps are 6s, so settling
/// past this means the interrupt was slept through.
const SETTLE_LIMIT: Duration = Duration::from_secs(20);

#[cfg(windows)]
mod windows_faces {
    use super::*;
    use rubash::api::{Flow, RunOptions, Session};
    use std::path::PathBuf;

    /// The real rubash binary beside the test binary (same derivation
    /// the engine's own respawn chain uses): the test binary lives in
    /// `<profile>/deps/`, the engine binary in `<profile>/`.
    fn rubash_exe() -> PathBuf {
        let current = std::env::current_exe().expect("test executable");
        let deps = current.parent().expect("deps dir");
        let profile = deps.parent().expect("profile dir");
        let candidate = profile.join("rubash.exe");
        if candidate.is_file() {
            return candidate;
        }
        PathBuf::from(env!("CARGO_BIN_EXE_rubash"))
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rubash-issue3467-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create issue3467 scratch dir");
        dir
    }

    /// A zero-byte sleep.exe makes `sleep` resolvable in PATH, which
    /// is what arms the in-process fast path
    /// (`emulated_external_stands_down`).
    fn dummy_sleep_bin_dir(tag: &str) -> PathBuf {
        let dir = scratch(&format!("bin-{tag}"));
        std::fs::write(dir.join("sleep.exe"), b"").expect("write dummy sleep.exe");
        dir
    }

    enum Inject {
        /// The new static face (the host's call shape).
        StaticFace,
        /// Tier-A session injection (the arms, face-independent).
        TierA,
    }

    struct DrivenRun {
        status: i32,
        flow: Flow,
        elapsed: Duration,
        notice: String,
        jobs_after: String,
        fg_status: Option<i32>,
        jobs_after_fg: String,
        alive: bool,
    }

    /// Run `command` (whose tail is the foreground wait) on an
    /// interactive-flagged session in a runner thread, with `set -m`
    /// (the fg gate, fg_bg.def:108-113 — fg refuses without the
    /// monitor option) and the session's stderr rebound via
    /// `exec 2>` to a notice file: the stop notice is a shell
    /// diagnostic on the shell's stderr, GNU-style, not the
    /// command's redirected stderr, so a per-command `2>` on the
    /// sleep would miss it. Once the handshake file exists (and
    /// the run has had a moment to block in its wait), queue an
    /// untrapped TSTP from the main thread and collect the run's
    /// outcome, the notice, the `jobs` listing, and — for the stop
    /// cases — the `fg` resume that follows in the same session.
    fn drive(command: &str, tag: &str, inject: Inject, resume_with_fg: bool) -> DrivenRun {
        let dir = scratch(&format!("run-{tag}"));
        let bin_dir = dummy_sleep_bin_dir(tag);
        let path = format!("{};C:\\windows\\system32;C:\\windows", bin_dir.display());
        let started: PathBuf = dir.join("started.txt");
        let notice: PathBuf = dir.join("notice.txt");
        let jobs_file: PathBuf = dir.join("jobs.txt");
        let jobs2_file: PathBuf = dir.join("jobs2.txt");
        let script = format!(
            "set -m; exec 2> \"{}\"; : > \"{}\"; {command}",
            notice.display(),
            started.display(),
        );
        let jobs_path = jobs_file.display().to_string();
        let jobs2_path = jobs2_file.display().to_string();

        let runner = std::thread::spawn(move || {
            let mut session = Session::builder()
                .env("__RUBASH_INTERACTIVE", "1")
                .env("PATH", &path)
                .build()
                .expect("interactive session builds");
            let mut options = RunOptions::default();
            options.interactive = true;
            let t0 = Instant::now();
            let outcome = session
                .run(&script, &options)
                .expect("the interrupted run is an outcome, not an error");
            let elapsed = t0.elapsed();
            let _ = session
                .run(&format!("jobs > \"{jobs_path}\""), &RunOptions::default())
                .expect("jobs run");
            let fg_status = if resume_with_fg {
                let fg = session.run("fg", &RunOptions::default()).expect("fg run");
                let _ = session
                    .run(&format!("jobs > \"{jobs2_path}\""), &RunOptions::default())
                    .expect("jobs-after-fg run");
                Some(fg.status)
            } else {
                None
            };
            let after = session
                .run("echo alive", &RunOptions::default())
                .expect("the session is usable after the interrupt");
            DrivenRun {
                status: outcome.status,
                flow: outcome.flow,
                elapsed,
                notice: std::fs::read_to_string(&notice).unwrap_or_default(),
                jobs_after: std::fs::read_to_string(&jobs_file).unwrap_or_default(),
                fg_status,
                jobs_after_fg: std::fs::read_to_string(&jobs2_file).unwrap_or_default(),
                alive: after.status == 0,
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
        // Settle before injecting: the queued signal must land while
        // the wait is established — a signal queued before the wait
        // is consumed by the pre-wait adjudication (the #698
        // forgotten-signal path) and never interrupts the sleep.
        // 400ms proved marginal on a loaded box (the unix pin
        // failed identically on pristine baseline); 1500ms gives
        // the runner room to spawn the child and enter its wait.
        std::thread::sleep(Duration::from_millis(1500));
        match inject {
            Inject::StaticFace => rubash::script_driver::inject_console_ctrl_z(),
            Inject::TierA => {
                let mut peer = Session::builder().build().expect("peer session builds");
                peer.inject_signal(20);
            }
        }

        let deadline = Instant::now() + SETTLE_LIMIT + Duration::from_secs(15);
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

    /// The #877 acceptance shape, through the new static face:
    /// foreground virtual `sleep` + ^Z stops the job (148, notice,
    /// `jobs` lists it) and `fg` resumes it to natural completion.
    #[test]
    fn virtual_sleep_ctrl_z_stops_job_and_fg_resumes_it() {
        let _guard = serial();
        let run = drive("sleep 6", "virt", Inject::StaticFace, true);
        assert_eq!(run.status, 148, "the wait's verdict is 128+20");
        assert_eq!(run.flow, Flow::Completed, "a stop is not an exit");
        assert!(
            run.elapsed < Duration::from_secs(5),
            "sleep ran to completion instead of stopping: {:?}",
            run.elapsed
        );
        assert!(run.alive, "session died on the ^Z");
        assert!(
            run.notice.contains("Stopped") && run.notice.contains("sleep 6"),
            "the stop notice must print: {:?}",
            run.notice
        );
        assert!(
            run.jobs_after.contains("Stopped") && run.jobs_after.contains("sleep 6"),
            "jobs must list the stopped sleep: {:?}",
            run.jobs_after
        );
        assert_eq!(
            run.fg_status,
            Some(0),
            "fg resumes the sleeper to natural completion"
        );
        assert!(
            !run.jobs_after_fg.contains("[1]"),
            "the resumed job leaves the table: {:?}",
            run.jobs_after_fg
        );
    }

    /// The real-child arm, through Tier-A injection: the outer shell
    /// spawns a nested rubash whose own `sleep 6` is the foreground
    /// child; ^Z suspends it (not kills it), `jobs` lists it, and
    /// `fg` resumes it to completion.
    #[test]
    fn real_child_ctrl_z_stops_job_and_fg_resumes_it() {
        let _guard = serial();
        let exe = rubash_exe();
        let command = format!("\"{}\" -c \"sleep 6\"", exe.display());
        let run = drive(&command, "real", Inject::TierA, true);
        assert_eq!(run.status, 148, "the wait's verdict is 128+20");
        assert_eq!(run.flow, Flow::Completed, "a stop is not an exit");
        assert!(
            run.elapsed < Duration::from_secs(5),
            "child ran to completion instead of stopping: {:?}",
            run.elapsed
        );
        assert!(run.alive, "session died on the ^Z");
        assert!(
            run.notice.contains("Stopped") && run.notice.contains("sleep 6"),
            "the stop notice must print: {:?}",
            run.notice
        );
        assert!(
            run.jobs_after.contains("Stopped"),
            "jobs must list the stopped child: {:?}",
            run.jobs_after
        );
        assert_eq!(
            run.fg_status,
            Some(0),
            "fg resumes the child to natural completion"
        );
        assert!(
            !run.jobs_after_fg.contains("[1]"),
            "the resumed job leaves the table: {:?}",
            run.jobs_after_fg
        );
    }

    /// A trapped TSTP is not a stop (GNU: a trapped SIGTSTP does not
    /// stop the shell): the action runs and the sleep completes.
    #[test]
    fn trapped_tstp_runs_trap_and_does_not_stop() {
        let _guard = serial();
        let dir = scratch("trap");
        let fired = dir.join("fired.txt");
        let fired_text = fired.display().to_string();
        let command = format!("trap 'echo FIRED > \"{fired_text}\"' TSTP; sleep 3");
        let run = drive(&command, "trap", Inject::StaticFace, false);
        assert_eq!(run.status, 0, "trapped TSTP must not stop the sleep");
        assert!(
            run.elapsed >= Duration::from_secs(2),
            "sleep ended early: {:?}",
            run.elapsed
        );
        assert!(run.alive, "session died on the ^Z");
        assert!(
            !run.jobs_after.contains("Stopped"),
            "no job may stop on a trapped TSTP: {:?}",
            run.jobs_after
        );
        assert!(
            std::fs::read_to_string(&fired)
                .unwrap_or_default()
                .contains("FIRED"),
            "the TSTP trap action never ran"
        );
    }
}

#[cfg(unix)]
mod unix_faces {
    use super::*;
    use rubash::api::{RunOptions, Session};

    /// The unix sleep is a real child since #3488: it still runs to
    /// natural completion with status 0.
    #[test]
    fn unix_sleep_still_completes() {
        let _guard = serial();
        let mut session = Session::builder().build().expect("session builds");
        let outcome = session
            .run("sleep 1; echo done", &RunOptions::default())
            .expect("run");
        assert_eq!(outcome.status, 0);
    }

    /// A self-queued TSTP on unix leaves a real-child sleep
    /// undisturbed: since #3488 the unix sleep is a real spawned
    /// child, and its wait is a blocking waitpid that never observes
    /// a mailbox TSTP (#3284) — only a real terminal ^Z delivered to
    /// the child's process group stops it, which is #2398's PTY
    /// territory. So the sleep runs to natural completion with
    /// status 0. (The Windows stop arm must not leak into this
    /// path.) An earlier draft of this pin asserted 148 here; that
    /// verdict belongs to the in-process virtual wait, which unix no
    /// longer uses for sleep — the 148 expectation failed identically
    /// on the pristine baseline.
    #[test]
    fn unix_queued_tstp_leaves_real_child_sleep_undisturbed() {
        let _guard = serial();
        let dir =
            std::env::temp_dir().join(format!("rubash-issue3467-unix-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create unix scratch dir");
        let started = dir.join("started.txt");
        let started_text = started.display().to_string();
        let script = format!(": > \"{started_text}\"; sleep 30");
        let runner = std::thread::spawn(move || {
            let mut session = Session::builder()
                .env("__RUBASH_INTERACTIVE", "1")
                .build()
                .expect("interactive session builds");
            let mut options = RunOptions::default();
            options.interactive = true;
            let t0 = Instant::now();
            let outcome = session.run(&script, &options).expect("run outcome");
            let elapsed = t0.elapsed();
            let after = session
                .run("echo alive", &RunOptions::default())
                .expect("session usable after");
            (outcome.status, elapsed, after.status)
        });
        let mut waited = Duration::ZERO;
        while !started.exists() {
            assert!(waited < Duration::from_secs(30), "run never started");
            std::thread::sleep(Duration::from_millis(25));
            waited += Duration::from_millis(25);
        }
        // Settle before injecting so the queued TSTP arrives
        // mid-wait, while the real child is already spawned and the
        // shell is inside its wait — not before the sleep starts.
        std::thread::sleep(Duration::from_millis(1500));
        let mut peer = Session::builder().build().expect("peer session builds");
        peer.inject_signal(20);
        let deadline = Instant::now() + SETTLE_LIMIT + Duration::from_secs(15);
        loop {
            if runner.is_finished() {
                break;
            }
            assert!(Instant::now() < deadline, "run never settled");
            std::thread::sleep(Duration::from_millis(50));
        }
        let (status, elapsed, alive) = runner.join().expect("runner thread");
        assert_eq!(
            status, 0,
            "queued TSTP must not disturb a unix real-child sleep"
        );
        assert!(
            elapsed >= Duration::from_secs(29),
            "sleep ended early, as if the queued TSTP unwound it: {:?}",
            elapsed
        );
        assert_eq!(alive, 0, "session died on the queued TSTP");
    }
}
