//! Issue rubash#2134 regressions: entry SIGINT/SIGQUIT ignores come from
//! actual async parentage, never from an ambient data marker.
//!
//! The defect: `__RUBASH_ASYNC_CHILD=1` in a fresh process's environment
//! was honored at face value by `apply_async_child_entry_ignores`, so any
//! script could export it and every shell it spawned was born ignoring
//! SIGINT/SIGQUIT (survives Ctrl+C; control exits 130). With `-i` the
//! forgery reached the interactive face — a shell the user cannot
//! interrupt. GNU reaches the ignore state only in a genuinely forked
//! async child (jobs.c setup_async_signals at the fork site); the
//! disposition is inherited through fork/exec, never asserted by data.
//!
//! The fix: the marker is scrubbed unconditionally, and the ignores are
//! applied only when (a) the process's respawn provenance validates
//! against its actual parent (rubash#2136) and (b) the invocation is a
//! `-c` respawn shape, not an interactive one.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const WATCHDOG: Duration = Duration::from_secs(20);

fn bin() -> String {
    env!("CARGO_BIN_EXE_rubash").replace('\\', "/")
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i2134-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

struct Run {
    stdout: String,
    code: Option<i32>,
}

fn spawn(args: &[&str], extra_env: &[(&str, &str)], stdin: &[u8]) -> Run {
    let dir = scratch("io");
    let in_path = dir.join("stdin");
    std::fs::write(&in_path, stdin).expect("write stdin");
    let mut command = Command::new(bin());
    command
        .args(args)
        .env("HOME", dir.to_string_lossy().replace('\\', "/"))
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .env_remove("__RUBASH_ASYNC_CHILD")
        .env_remove("__RUBASH_SHELL_PID")
        .env_remove("__RUBASH_COPROC_CHILD")
        .env_remove("__RUBASH_SPAWNER_PID")
        .stdin(Stdio::from(
            std::fs::File::open(&in_path).expect("stdin file"),
        ))
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let mut child = command.spawn().expect("spawn rubash");
    let deadline = Instant::now() + WATCHDOG;
    let code = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => {
                // Shell-convention exit status: 128 + signal when the
                // process died by a signal (status.code() is None then).
                // Death-by-signal is a unix wait-status concept; a
                // Windows exit status always carries an exit code.
                #[cfg(unix)]
                let code = match status.code() {
                    Some(code) => Some(code),
                    None => {
                        use std::os::unix::process::ExitStatusExt;
                        Some(128 + status.signal().unwrap_or(0))
                    }
                };
                #[cfg(not(unix))]
                let code = status.code();
                break code;
            }
            None => {
                assert!(Instant::now() < deadline, "watchdog: child hung");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    };
    let mut stdout = String::new();
    if let Some(mut out) = child.stdout.take() {
        use std::io::Read;
        let _ = out.read_to_string(&mut stdout);
    }
    Run {
        stdout: stdout.trim().to_string(),
        code,
    }
}

/// Self-INT probe: an unignored SIGINT delivered to the shell's own real
/// pid (BASHPID) kills it with 130 before the echo; an ignored one lets
/// the echo print.
const SELF_INT: &str = "kill -INT $BASHPID; echo SURVIVED_INT";

#[test]
fn forged_async_marker_does_not_ignore_sigint() {
    let run = spawn(&["-c", SELF_INT], &[("__RUBASH_ASYNC_CHILD", "1")], b"");
    assert_eq!(run.code, Some(130), "forged marker must stay inert");
    assert!(!run.stdout.contains("SURVIVED_INT"));
}

#[test]
fn control_shell_dies_on_self_int() {
    let run = spawn(&["-c", SELF_INT], &[], b"");
    assert_eq!(run.code, Some(130));
    assert!(!run.stdout.contains("SURVIVED_INT"));
}

#[test]
fn forged_marker_with_stranger_spawner_stamp_still_dies() {
    // Even a complete-looking forgery (marker + claim + a spawner stamp
    // naming a dead stranger) fails the parentage check.
    let mut dead = Command::new(bin())
        .args(["-c", "exit"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn short-lived rubash");
    let dead_pid = dead.id().to_string();
    let _ = dead.wait();
    let run = spawn(
        &["-c", SELF_INT],
        &[
            ("__RUBASH_ASYNC_CHILD", "1"),
            ("__RUBASH_SHELL_PID", "424242"),
            ("__RUBASH_SPAWNER_PID", &dead_pid),
        ],
        b"",
    );
    assert_eq!(run.code, Some(130));
    assert!(!run.stdout.contains("SURVIVED_INT"));
}

#[test]
fn validated_parentage_applies_the_ignores() {
    // The test process IS the child's actual parent, so a claim stamped
    // with its pid validates — this is the deliberate-parent class (the
    // parent could equally have pre-ignored the dispositions before
    // spawn, which fork/exec would inherit). The `-c` child must behave
    // exactly like a genuine async respawn at the disposition level.
    let own = std::process::id().to_string();
    let run = spawn(
        &["-c", SELF_INT],
        &[
            ("__RUBASH_ASYNC_CHILD", "1"),
            ("__RUBASH_SHELL_PID", &own),
            ("__RUBASH_SPAWNER_PID", &own),
        ],
        b"",
    );
    assert_eq!(run.code, Some(0));
    assert!(run.stdout.contains("SURVIVED_INT"));
}

#[test]
fn interactive_shape_refuses_even_validated_provenance() {
    // Same validated provenance as above, but an interactive-shaped
    // invocation: the ignores must NOT be applied — an interactive
    // shell is never born ignoring Ctrl+C.
    let own = std::process::id().to_string();
    let run = spawn(
        &["-i", "-c", SELF_INT],
        &[
            ("__RUBASH_ASYNC_CHILD", "1"),
            ("__RUBASH_SHELL_PID", &own),
            ("__RUBASH_SPAWNER_PID", &own),
        ],
        b"",
    );
    assert_eq!(run.code, Some(130));
    assert!(!run.stdout.contains("SURVIVED_INT"));
}

#[test]
fn stdin_shaped_invocation_refuses_even_validated_provenance() {
    // No -c, no script operand: commands arrive on stdin (the session
    // shape). Same refusal as the -i shape.
    let own = std::process::id().to_string();
    let run = spawn(
        &[],
        &[
            ("__RUBASH_ASYNC_CHILD", "1"),
            ("__RUBASH_SHELL_PID", &own),
            ("__RUBASH_SPAWNER_PID", &own),
        ],
        b"kill -INT $BASHPID; echo SURVIVED_INT\n",
    );
    assert_eq!(run.code, Some(130));
    assert!(!run.stdout.contains("SURVIVED_INT"));
}

#[test]
fn genuine_async_child_keeps_its_ignores() {
    // GNU jobs.c setup_async_signals: a genuinely forked async child
    // (job control off) is born ignoring SIGINT/SIGQUIT and survives
    // both, while the parent shell completes normally.
    let run = spawn(
        &[
            "-c",
            "( kill -INT $BASHPID; kill -QUIT $BASHPID; echo SURVIVED_BOTH ) & wait; echo OUTER_DONE",
        ],
        &[],
        b"",
    );
    assert_eq!(run.code, Some(0));
    assert!(run.stdout.contains("SURVIVED_BOTH"), "{}", run.stdout);
    assert!(run.stdout.contains("OUTER_DONE"), "{}", run.stdout);
}
