//! niubash#977 (engine half) — `BindXRunOutcome::interrupted` must
//! report, exactly, that a `bind -x` widget run was unwound by an
//! UNTRAPPED SIGINT, so the host can discard the edit line the way
//! GNU does. During the widget the tty is cooked and the ^C byte
//! never reaches the line editor; the shell owns SIGINT for the
//! duration (bashline.c:4603 bash_execute_unix_command clears
//! readline's handlers and parse_and_executes the command,
//! bashline.c:4688), and GNU's verdicts are disposition-shaped:
//!
//!   untrapped: the foreground job dies by SIGINT, waitchld does
//!     ADDINTERRUPT (jobs.c:3316), the list throws to the top-level
//!     reader (DISCARD; eval.c's parse_and_execute DISCARD arm
//!     leaves $? = EXECUTION_FAILURE when the status was 0) — the
//!     host must discard the line.
//!   trapped: the job still dies, the command list CONTINUES, the
//!     trap runs exactly once at the next boundary — the line is
//!     KEPT, so the flag must stay false.
//!   ignored (`trap '' INT`): the delivery is a no-op end to end —
//!     flag false, widget runs to completion.
//!
//! The returned `status` cannot carry the untrapped fact on its
//! own (an ordinary failure and a discard share statuses), which
//! is why the outcome carries the flag, recorded by the engine at
//! the unit's unwind-capture point (the Q6 arm B unit
//! signal-discard record), never re-derived by the host.
//!
//! Delivery discipline (the issue698 / issue1639 in-process
//! rules): the delivery queue and the kernel-signal backend are
//! process-global, so every cell serializes on one mutex, each
//! scenario builds its interactive executor on a runner thread,
//! and the interrupt is a real kill(2) SIGINT to this process
//! while the widget's external sleep is in its foreground wait —
//! the engine's sliced wait forwards the recorded delivery to the
//! child (rubash#1990), whose SIGINT death is the verdict
//! (rubash#1639). Unix only: the mailbox faces are niu's console
//! route and are pinned there.

#![cfg(unix)]

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rubash::executor::Executor;
use rubash::script_driver::run_source;

static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn external_sleep() -> &'static str {
    if std::path::Path::new("/bin/sleep").exists() {
        "/bin/sleep"
    } else {
        "/usr/bin/sleep"
    }
}

struct Driven {
    status: i32,
    interrupted: bool,
    log: String,
    elapsed: Duration,
}

/// Run one widget scenario. `setup` runs first on the scenario's
/// own interactive executor (trap declarations); the widget is a
/// function `w977` whose body is `widget_body` — it drops the
/// handshake file first, so a delivering caller knows the run is
/// inside the widget before it sends the SIGINT. A settle unit
/// runs on the same executor before the log is read, so a pending
/// trap (and any leftover delivery) is adjudicated by THIS
/// scenario, never by the next one.
fn drive(tag: &str, setup: &str, widget_body: &str, deliver: bool) -> Driven {
    let dir = std::env::temp_dir().join(format!("rubash-issue977-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue977 scratch dir");
    let started: PathBuf = dir.join("started.txt");
    let log: PathBuf = dir.join("log.txt");
    let started_text = started.display().to_string();
    let log_text = log.display().to_string();
    let setup = setup.replace("{LOG}", &log_text);
    let widget_def = format!(
        "w977() {{ : > \"{started_text}\"; {} }}",
        widget_body
            .replace("{SLEEP}", external_sleep())
            .replace("{LOG}", &log_text)
    );
    let log_for_thread = log.clone();

    let runner = std::thread::spawn(move || {
        let mut executor = Executor::new();
        executor.set_env("INPUTRC", "/dev/null");
        executor.set_invocation_shape(true, false);
        // Warm-up: adjudicate any stray recorded delivery against a
        // throwaway unit before this scenario's own state exists.
        let _ = run_source(&mut executor, "true", false);
        if !setup.is_empty() {
            assert_eq!(
                run_source(&mut executor, &setup, false),
                0,
                "setup unit failed"
            );
        }
        assert_eq!(
            run_source(&mut executor, &widget_def, false),
            0,
            "widget definition failed"
        );
        let t0 = Instant::now();
        let outcome = executor.run_bind_x_command("w977", "zz", 2, 0);
        let elapsed = t0.elapsed();
        let _ = run_source(&mut executor, "true", false);
        Driven {
            status: outcome.status,
            interrupted: outcome.interrupted,
            log: std::fs::read_to_string(&log_for_thread).unwrap_or_default(),
            elapsed,
        }
    });

    if deliver {
        let mut waited = Duration::ZERO;
        while !started.exists() {
            assert!(waited < Duration::from_secs(30), "the widget never started");
            std::thread::sleep(Duration::from_millis(25));
            waited += Duration::from_millis(25);
        }
        // Let the widget settle into its foreground wait, then
        // deliver a real SIGINT to this process (the shell's pid —
        // the foreground child is reached by the wait's forwarding,
        // rubash#1990, exactly as a group delivery would kill it).
        std::thread::sleep(Duration::from_millis(500));
        let rc = unsafe { libc::kill(std::process::id() as i32, libc::SIGINT) };
        assert_eq!(rc, 0, "kill(SIGINT) failed");
    }
    runner.join().expect("runner thread")
}

/// The defect cell: untrapped SIGINT during the widget. The
/// external sleep dies by SIGINT, the rest of the widget never
/// runs, and the outcome must say so.
#[test]
fn untrapped_sigint_during_widget_marks_interrupted() {
    let _guard = serial();
    let run = drive("untrapped", "", "{SLEEP} 15; echo done >> \"{LOG}\"", true);
    assert!(
        run.interrupted,
        "an untrapped SIGINT unwind during the widget must set interrupted \
         (status {}, log {:?})",
        run.status, run.log
    );
    assert_eq!(run.status, 130, "the discard verdict carries 130");
    assert!(
        !run.log.contains("done"),
        "the widget's post-sleep commands must not run: {:?}",
        run.log
    );
    assert!(
        run.elapsed < Duration::from_secs(12),
        "the 15s sleep must have been cut by the interrupt: {:?}",
        run.elapsed
    );
}

/// The trapped control: the sleep still ends (here: runs out —
/// the pid-directed delivery never reaches it), the widget
/// completes, the trap runs exactly once, and the flag stays
/// false so the host KEEPS the line.
#[test]
fn trapped_sigint_during_widget_runs_trap_once_and_does_not_mark() {
    let _guard = serial();
    let run = drive(
        "trapped",
        "trap 'echo trapped >> \"{LOG}\"' INT",
        "{SLEEP} 2; echo done >> \"{LOG}\"",
        true,
    );
    assert!(
        !run.interrupted,
        "a trapped SIGINT must not mark the run interrupted"
    );
    assert_eq!(run.status, 0, "the widget completes normally");
    assert!(
        run.log.contains("done"),
        "the widget must run to completion: {:?}",
        run.log
    );
    assert_eq!(
        run.log.matches("trapped").count(),
        1,
        "the INT trap must run exactly once: {:?}",
        run.log
    );
}

/// The ignored control: `trap '' INT` makes the delivery a
/// no-op — the sleep is not cut, the widget completes, the flag
/// stays false.
#[test]
fn ignored_sigint_during_widget_is_a_no_op() {
    let _guard = serial();
    let run = drive(
        "ignored",
        "trap '' INT",
        "{SLEEP} 2; echo done >> \"{LOG}\"",
        true,
    );
    assert!(
        !run.interrupted,
        "an ignored SIGINT must not mark the run interrupted"
    );
    assert_eq!(run.status, 0);
    assert!(
        run.log.contains("done"),
        "the widget must run to completion: {:?}",
        run.log
    );
    assert!(
        run.elapsed >= Duration::from_millis(1800),
        "the ignored delivery must not cut the 2s sleep: {:?}",
        run.elapsed
    );
}

/// The quiet control: no delivery at all — a normally completing
/// widget is not interrupted.
#[test]
fn widget_without_signal_is_not_interrupted() {
    let _guard = serial();
    let run = drive("quiet", "", "echo done >> \"{LOG}\"", false);
    assert!(!run.interrupted);
    assert_eq!(run.status, 0);
    assert!(run.log.contains("done"), "log: {:?}", run.log);
}

/// The builtin-sleep spelling of the same cell: the widget's
/// `sleep` is the in-process virtual child, whose interrupt
/// arrives through the signal drains rather than a foreground
/// wait. The verdict must be the same — interrupted, and the
/// rest of the widget discarded.
#[test]
fn untrapped_sigint_during_builtin_sleep_widget_marks_interrupted() {
    let _guard = serial();
    let run = drive("builtin-sleep", "", "sleep 4; echo done >> \"{LOG}\"", true);
    assert!(
        run.interrupted,
        "an untrapped SIGINT during the builtin sleep must set interrupted \
         (status {}, log {:?}, elapsed {:?})",
        run.status, run.log, run.elapsed
    );
    assert!(
        !run.log.contains("done"),
        "the widget's post-sleep commands must not run: {:?}",
        run.log
    );
}
