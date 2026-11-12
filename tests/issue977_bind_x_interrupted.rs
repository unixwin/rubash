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
//! while the widget is in its foreground wait. The delivery is
//! addressed to the shell's pid only: GNU remembers such an
//! interrupt across the wait and discards the list when the
//! child exits (rubash#1640; the process-group shape is pinned
//! at the binary level by the issue1639 PTY cells). Unix only:
//! the mailbox faces are niu's console route and are pinned
//! there.

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

/// True when an external `sleep` child of this process exists —
/// i.e. the widget has spawned its foreground child and the
/// executor is in (or one slice from) its foreground wait
/// (`wait_fg_child_draining` / `wait_fg_child_sliced`,
/// external_inner.rs). Read from /proc, so on unix targets
/// without /proc this is always false and the caller falls back
/// to a longer fixed settle.
fn foreground_sleep_child_running() -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    let me = std::process::id();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Ok(pid) = name.to_str().unwrap_or("").parse::<u32>() else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        // stat is `pid (comm) state ppid ...`; comm may contain
        // spaces, so the fields after it are split off the last ") ".
        let Some(after) = stat.rsplit(") ").next() else {
            continue;
        };
        let mut fields = after.split_whitespace();
        let _state = fields.next();
        let Ok(ppid) = fields.next().unwrap_or("").parse::<u32>() else {
            continue;
        };
        if ppid != me {
            continue;
        }
        let Some(open) = stat.find('(') else { continue };
        let Some(close) = stat.rfind(')') else {
            continue;
        };
        if close > open && &stat[open + 1..close] == "sleep" {
            return true;
        }
    }
    false
}

/// Wait for the widget's foreground wait, then let the wait loop
/// arm, before the caller delivers SIGINT (rubash#3289). The poll
/// is bounded so the delivery still lands inside even the
/// shortest widget sleep (the trapped/ignored cells sleep 4s):
/// 2s of polling + a 250ms arming grace, then deliver regardless.
/// The builtin-sleep cell has no external child, so its poll
/// always runs out — the delivery then lands ~2s into its 4s
/// sleep, still mid-wait where the fold consumes it. Without
/// /proc the poll never observes a child and the same bound
/// degrades to a longer fixed settle (2s, 4x the old 500ms).
fn wait_for_foreground_wait() {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if foreground_sleep_child_running() {
            // The child is spawned; give the executor's wait loop
            // a scheduling quantum to enter its drain before the
            // delivery lands.
            std::thread::sleep(Duration::from_millis(250));
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
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
        "w977() {{ : > \"{started_text}\"; {} ; }}",
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
        // rubash#3289: wait for the widget to reach its foreground
        // wait before delivering, instead of a blind settle. The
        // old 500ms sleep from the handshake file flaked under box
        // load: the runner thread can sit unscheduled for longer
        // than 500ms between creating `started.txt` and spawning
        // the child, and a SIGINT delivered in that window is never
        // registered (trap count 0, no unwind). Delivering only
        // once the child exists closes the whole window — the
        // executor has spawned the foreground child and is in (or
        // one slice from) its wait loop, where the drain records
        // the delivery.
        wait_for_foreground_wait();
        // Deliver a real SIGINT to this process (the shell's pid —
        // the foreground child is reached by the wait's forwarding,
        // rubash#1990, exactly as a group delivery would kill it).
        let rc = unsafe { libc::kill(std::process::id() as i32, libc::SIGINT) };
        assert_eq!(rc, 0, "kill(SIGINT) failed");
    }
    runner.join().expect("runner thread")
}

/// The defect cell: untrapped SIGINT during the widget. The
/// delivery here is addressed to the shell's pid only, so the
/// external child never receives it and the wait runs out —
/// GNU remembers a shell-directed interrupt across the wait
/// and discards the rest of the line when the child exits
/// (rubash#1640 readjudication, pinned at the binary level by
/// the issue1639 PTY cells; the instant-death shape belongs to
/// terminal group delivery, which an in-process test cannot
/// safely aim at its own process group). The contract under
/// test is the discard itself: the rest of the widget never
/// runs, the status is 130, and the outcome must say so.
#[test]
fn untrapped_sigint_during_widget_marks_interrupted() {
    let _guard = serial();
    let run = drive("untrapped", "", "{SLEEP} 6; echo done >> \"{LOG}\"", true);
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
        run.elapsed >= Duration::from_millis(5500),
        "a shell-directed delivery must not cut the child's 6s sleep; \
         the discard lands when the wait ends: {:?}",
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
        "{SLEEP} 4; echo done >> \"{LOG}\"",
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
        "{SLEEP} 4; echo done >> \"{LOG}\"",
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
        run.elapsed >= Duration::from_millis(3800),
        "the ignored delivery must not cut the 4s sleep: {:?}",
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

/// The builtin-sleep boundary: the widget's `sleep` is the
/// in-process virtual child, and a delivery that lands
/// mid-sleep is consumed by the mid-wait drain and FORGOTTEN
/// with its classification — the virtual child cannot die, so
/// a shell-directed delivery neither cuts the sleep nor
/// discards the list (rubash#1640, pinned by
/// issue698_forgotten_mid_sleep_sigint_leaves_no_discard_mark;
/// re-adjudicating that fold is #1640's own question, not this
/// flag's). No unwind occurs in this spelling, so the flag
/// must stay false and the widget completes: this cell locks
/// the boundary between the two mechanisms, so a change to
/// either side has to move it consciously.
#[test]
fn builtin_sleep_mid_wait_fold_leaves_no_unwind_to_report() {
    let _guard = serial();
    let run = drive("builtin-sleep", "", "sleep 4; echo done >> \"{LOG}\"", true);
    assert!(
        !run.interrupted,
        "the mid-wait fold produces no unwind to report \
         (status {}, log {:?}, elapsed {:?})",
        run.status, run.log, run.elapsed
    );
    assert_eq!(run.status, 0);
    assert!(
        run.log.contains("done"),
        "the widget must run to completion under the fold: {:?}",
        run.log
    );
}
