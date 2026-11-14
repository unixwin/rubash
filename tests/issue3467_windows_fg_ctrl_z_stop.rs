//! rubash#3467 — the engine stop surface for a Windows foreground Ctrl+Z
//! (the engine half of niubash#877).
//!
//! GNU: the terminal driver turns ^Z into SIGTSTP for the terminal's
//! foreground process group — under job control the foreground JOB's
//! group (jobs.c:2400 make_child -> give_terminal_to), never the
//! shell, which ignores SIGTSTP for itself (jobs.c:4949-4955
//! initialize_job_signals). waitchld waits with WUNTRACED
//! (jobs.c:4077), so the stop is a wait outcome: the job goes Stopped
//! and current (jobs.c:4290 set_job_status_and_cleanup, :4338), the
//! wait settles at 128 + WSTOPSIG (jobs.c:3220-3221), `jobs` lists
//! `Stopped`, `fg`/`bg` resume it (jobs.c:3826 start_job).
//!
//! Windows has no SIGTSTP, so the host reports the console ^Z through
//! `script_driver::inject_console_ctrl_z()`, and the engine's stoppable
//! foreground waits (an external child; the `fg` wait) suspend the
//! job and settle it exactly that way. Pre-#3467 the face did not
//! exist at all (this binary did not compile), and the only nearby
//! surface — pushing 20 into the shell's signal mailbox — ended the
//! wait at 148 by KILLING the child (no job, nothing for fg).
//!
//! The channel is process-global, so every test serializes on one
//! mutex; each scenario drives one interactive session on a worker
//! thread (the session outlives several runs: stop, then fg/bg) while
//! the test thread performs the host's injection, as niu's
//! ForegroundWatch will.

use std::sync::Mutex;

static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Unix: the kernel delivers the terminal's SIGTSTP to the foreground
/// group itself; the host face is a no-op that reports "not armed".
#[cfg(unix)]
#[test]
fn issue3467_unix_inject_console_ctrl_z_is_a_noop() {
    let _guard = serial();
    assert!(!rubash::script_driver::inject_console_ctrl_z());
}

#[cfg(windows)]
mod windows_faces {
    use super::serial;
    use rubash::api::{RunOptions, Session};
    use std::path::{Path, PathBuf};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    /// A ping that runs for ~`n - 1` seconds: the #2020/#698 probe
    /// vehicle (a single-process external foreground command).
    fn ping(n: u32) -> String {
        format!("ping -n {n} 127.0.0.1 > ping.out")
    }

    struct Worker {
        scripts: mpsc::Sender<String>,
        outcomes: mpsc::Receiver<(i32, Duration)>,
        dir: PathBuf,
    }

    impl Worker {
        /// One interactive session (the niu host shape: interactive
        /// flag + interactive runs) living on its own thread, its cwd
        /// in a scratch dir and fd 2 bound to `stderr.txt` there so
        /// the stop notice is observable.
        fn start(tag: &str) -> Worker {
            let dir = std::env::temp_dir().join(format!(
                "rubash-issue3467-{}-{}",
                std::process::id(),
                tag
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("create issue3467 scratch dir");
            let (script_tx, script_rx) = mpsc::channel::<String>();
            let (outcome_tx, outcome_rx) = mpsc::channel::<(i32, Duration)>();
            let cd = format!(
                "cd '{}' && exec 2> stderr.txt",
                dir.display().to_string().replace('\\', "/")
            );
            std::thread::spawn(move || {
                let mut session = Session::builder()
                    .env("__RUBASH_INTERACTIVE", "1")
                    .build()
                    .expect("interactive session builds");
                let mut options = RunOptions::default();
                options.interactive = true;
                session.run(&cd, &options).expect("scratch setup runs");
                for script in script_rx {
                    let t0 = Instant::now();
                    let status = session
                        .run(&script, &options)
                        .map(|outcome| outcome.status)
                        .unwrap_or(-1);
                    if outcome_tx.send((status, t0.elapsed())).is_err() {
                        break;
                    }
                }
            });
            Worker {
                scripts: script_tx,
                outcomes: outcome_rx,
                dir,
            }
        }

        fn submit(&self, script: &str) {
            let _ = std::fs::remove_file(self.dir.join("started"));
            self.scripts
                .send(format!(": > started; {script}"))
                .expect("worker alive");
        }

        fn collect(&self, limit: Duration) -> (i32, Duration) {
            self.outcomes
                .recv_timeout(limit)
                .expect("the run settled within its limit")
        }

        fn run(&self, script: &str) -> (i32, Duration) {
            self.submit(script);
            self.collect(Duration::from_secs(30))
        }

        /// Submit `script`, wait until it reached its foreground wait,
        /// then inject the console ^Z the way the host does.
        fn run_and_ctrl_z(&self, script: &str) -> (bool, i32, Duration) {
            self.submit(script);
            let started = self.dir.join("started");
            let t0 = Instant::now();
            while !started.exists() {
                assert!(
                    t0.elapsed() < Duration::from_secs(30),
                    "the run never started"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
            std::thread::sleep(Duration::from_millis(600));
            let accepted = rubash::script_driver::inject_console_ctrl_z();
            let (status, elapsed) = self.collect(Duration::from_secs(30));
            (accepted, status, elapsed)
        }

        fn read(&self, name: &str) -> String {
            read_file(&self.dir.join(name))
        }
    }

    fn read_file(path: &Path) -> String {
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .replace("\r\n", "\n")
    }

    /// The #877 acceptance chain on an external foreground command:
    /// ^Z stops it (148, `jobs` reports Stopped, the GNU stop notice
    /// on fd 2), `fg` resumes it to completion (its own status, the
    /// job leaves the table).
    #[test]
    fn issue3467_ctrl_z_stops_foreground_external_and_fg_resumes_it() {
        let _guard = serial();
        let worker = Worker::start("stop-fg");
        worker.run("set -m");
        let (accepted, status, elapsed) =
            worker.run_and_ctrl_z(&format!("{}; echo \"rc=$?\" > rc1; jobs > jobs1", ping(8)));
        assert!(accepted, "a stoppable foreground wait was armed");
        assert!(
            elapsed < Duration::from_secs(6),
            "the stop settled the wait at once, not at the child's end: {elapsed:?}"
        );
        assert_eq!(status, 0, "the list continued after the stop");
        assert_eq!(worker.read("rc1"), "rc=148\n", "128 + SIGTSTP");
        assert_eq!(
            worker.read("jobs1"),
            "[1]+  Stopped                    ping -n 8 127.0.0.1 > ping.out\n"
        );
        assert_eq!(
            worker.read("stderr.txt"),
            "\n[1]+  Stopped                    ping -n 8 127.0.0.1 > ping.out\n",
            "jobs.c:4690-4691 notify_of_job_status stop notice"
        );

        let (status, _) = worker.run("fg > fg.out; echo \"rc=$?\" > rc2; jobs > jobs2");
        assert_eq!(status, 0);
        assert_eq!(worker.read("fg.out"), "ping -n 8 127.0.0.1 > ping.out\n");
        assert_eq!(worker.read("rc2"), "rc=0\n", "the resumed ping completed");
        assert_eq!(worker.read("jobs2"), "", "fg consumed the job");
        assert!(
            !worker.read("ping.out").is_empty(),
            "the resumed child ran to its end"
        );
    }

    /// `bg` resumes the stopped job in the background; `wait` reaps it
    /// with its own status.
    #[test]
    fn issue3467_ctrl_z_stopped_job_resumes_with_bg() {
        let _guard = serial();
        let worker = Worker::start("stop-bg");
        worker.run("set -m");
        let (accepted, _, _) = worker.run_and_ctrl_z(&format!("{}; echo \"rc=$?\" > rc1", ping(6)));
        assert!(accepted);
        assert_eq!(worker.read("rc1"), "rc=148\n");
        let (status, elapsed) =
            worker.run("bg > bg.out; jobs > jobs2; wait %1; echo \"rc=$?\" > rc2");
        assert_eq!(status, 0);
        assert!(elapsed < Duration::from_secs(20), "{elapsed:?}");
        assert_eq!(
            worker.read("bg.out"),
            "[1]+ ping -n 6 127.0.0.1 > ping.out &\n"
        );
        assert_eq!(
            worker.read("jobs2"),
            "[1]+  Running                    ping -n 6 127.0.0.1 > ping.out &\n"
        );
        assert_eq!(worker.read("rc2"), "rc=0\n");
    }

    /// A ^Z while `fg` holds the job re-stops it (the unix fg wait's
    /// WUNTRACED re-stop): fg settles at 148, the job is Stopped again,
    /// and a second fg completes it.
    #[test]
    fn issue3467_ctrl_z_restops_job_under_fg() {
        let _guard = serial();
        let worker = Worker::start("restop");
        worker.run("set -m");
        let (accepted, _, _) =
            worker.run_and_ctrl_z(&format!("{}; echo \"rc=$?\" > rc1", ping(10)));
        assert!(accepted);
        assert_eq!(worker.read("rc1"), "rc=148\n");
        let (accepted, _, elapsed) =
            worker.run_and_ctrl_z("fg > fg.out; echo \"rc=$?\" > rc2; jobs > jobs2");
        assert!(accepted, "fg's wait is armed for the re-stop");
        assert!(elapsed < Duration::from_secs(6), "{elapsed:?}");
        assert_eq!(worker.read("rc2"), "rc=148\n");
        assert_eq!(
            worker.read("jobs2"),
            "[1]+  Stopped                    ping -n 10 127.0.0.1 > ping.out\n"
        );
        let (_, _) = worker.run("fg > fg.out; echo \"rc=$?\" > rc3; jobs > jobs3");
        assert_eq!(worker.read("rc3"), "rc=0\n");
        assert_eq!(worker.read("jobs3"), "");
    }

    /// Without job control the channel is not live: GNU's waitchld does
    /// not wait WUNTRACED (jobs.c:4077), so there is no stop for the
    /// shell to report. The request is refused and the command runs to
    /// its natural end with its own status.
    #[test]
    fn issue3467_ctrl_z_without_monitor_is_refused() {
        let _guard = serial();
        let worker = Worker::start("no-monitor");
        worker.run("set +m");
        let (accepted, _, elapsed) =
            worker.run_and_ctrl_z(&format!("{}; echo \"rc=$?\" > rc1; jobs > jobs1", ping(3)));
        assert!(!accepted, "no stoppable wait is armed with monitor off");
        assert!(elapsed >= Duration::from_millis(1500), "{elapsed:?}");
        assert_eq!(worker.read("rc1"), "rc=0\n");
        assert_eq!(worker.read("jobs1"), "");
    }

    /// Outside a stoppable wait the shell itself is the terminal's
    /// foreground: GNU's SIG_IGN drops the keystroke. A request made
    /// then is refused and can never stop a LATER command.
    #[test]
    fn issue3467_ctrl_z_outside_a_wait_is_dropped() {
        let _guard = serial();
        assert!(!rubash::script_driver::inject_console_ctrl_z());
        let worker = Worker::start("dropped");
        worker.run("set -m");
        assert!(!rubash::script_driver::inject_console_ctrl_z());
        let (status, elapsed) = worker.run(&format!("{}; echo \"rc=$?\" > rc1", ping(3)));
        assert_eq!(status, 0);
        assert!(elapsed >= Duration::from_millis(1500), "{elapsed:?}");
        assert_eq!(worker.read("rc1"), "rc=0\n");
    }

    /// The request never reaches the shell's own trap table: a TSTP
    /// trap in the shell does not fire for the console ^Z (the shell is
    /// not in the job's group), the job stops all the same.
    #[test]
    fn issue3467_ctrl_z_bypasses_shell_tstp_trap() {
        let _guard = serial();
        let worker = Worker::start("trap");
        worker.run("set -m; trap 'echo trapped >> trap.out' TSTP");
        let (accepted, _, _) = worker.run_and_ctrl_z(&format!("{}; echo \"rc=$?\" > rc1", ping(6)));
        assert!(accepted);
        assert_eq!(worker.read("rc1"), "rc=148\n");
        worker.run(": boundary");
        assert_eq!(worker.read("trap.out"), "", "the shell's trap stays quiet");
        worker.run("kill %1");
    }
}
