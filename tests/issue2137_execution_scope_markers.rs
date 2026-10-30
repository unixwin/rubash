//! Issue rubash#2137 regressions: execution-scoped markers are not
//! import state, and POSIX mode has exactly one source of truth.
//!
//! F7 — `__RUBASH_DEBUGGER_MODE` ambient: every `-c` child attempted a
//! debugger startup (and the ATTEMPTED latch then suppressed the real
//! thing). The only legitimate writers are the process's own argv
//! handling (invocation.rs `--debugger`, main.rs option arms), which run
//! after import — so both keys are stripped at import.
//!
//! F8 — `__RUBASH_COMSUB_BODY` ambient: the marker is bracketed
//! in-process state around an embedded command-substitution run; an
//! ambient copy rewrote a top-level fatal expansion's exit status from
//! 127 to 1 (parameter_errors.rs `expansion_fatal_status`). Stripped at
//! import; the in-process writers re-establish it per run.
//!
//! F9 — POSIX split state: `posix_mode_enabled()` and the dialect read
//! sites consult the `__RUBASH_POSIX_MODE` marker while `set -o` reports
//! the `__RUBASH_SETOPT_posix` key, and both halves were
//! ambient-writable, so they could disagree in both directions. At
//! import, a validated respawn (rubash#2136) re-projects the marker from
//! the transported option truth; a top-level entry drops the marker and
//! the transported SETOPT family entirely, and the GNU channels
//! (POSIXLY_CORRECT, SHELLOPTS/BASHOPTS, `set -o posix`,
//! apply_posix_mode_transition) re-derive the pair in sync.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const WATCHDOG: Duration = Duration::from_secs(20);

fn bin() -> String {
    env!("CARGO_BIN_EXE_rubash").replace('\\', "/")
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i2137-{tag}-{}-{}",
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
    stderr: String,
    code: Option<i32>,
}

fn spawn(args: &[&str], extra_env: &[(&str, &str)], stdin: &[u8]) -> Run {
    let dir = scratch("io");
    let in_path = dir.join("stdin");
    let err_path = dir.join("stderr");
    std::fs::write(&in_path, stdin).expect("write stdin");
    let stderr_file = std::fs::File::create(&err_path).expect("stderr file");
    let mut command = Command::new(bin());
    command
        .args(args)
        .env("HOME", dir.to_string_lossy().replace('\\', "/"))
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .env_remove("__RUBASH_DEBUGGER_MODE")
        .env_remove("__RUBASH_DEBUGGER_ATTEMPTED")
        .env_remove("__RUBASH_COMSUB_BODY")
        .env_remove("__RUBASH_POSIX_MODE")
        .env_remove("__RUBASH_SETOPT_posix")
        .env_remove("POSIXLY_CORRECT")
        .stdin(Stdio::from(
            std::fs::File::open(&in_path).expect("stdin file"),
        ))
        .stdout(Stdio::piped())
        .stderr(Stdio::from(stderr_file));
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let mut child = command.spawn().expect("spawn rubash");
    let deadline = Instant::now() + WATCHDOG;
    let code = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break Some(status.code().unwrap_or(-1)),
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
        stderr: std::fs::read_to_string(&err_path).unwrap_or_default(),
        code,
    }
}

fn run_c(source: &str, extra_env: &[(&str, &str)]) -> Run {
    spawn(&["-c", source], extra_env, b"")
}

fn posix_option(source: &str, extra_env: &[(&str, &str)]) -> String {
    let run = run_c(source, extra_env);
    run.stdout
        .lines()
        .find(|line| line.starts_with("posix"))
        .map(|line| line.split_whitespace().last().unwrap_or("").to_string())
        .unwrap_or_else(|| format!("<missing in {:?}>", run.stdout))
}

// ---------------------------------------------------------------- F7 ---

#[test]
fn ambient_debugger_mode_does_not_start_a_debugger() {
    let run = run_c("echo RAN", &[("__RUBASH_DEBUGGER_MODE", "1")]);
    assert_eq!(run.code, Some(0));
    assert_eq!(run.stdout, "RAN");
    assert!(
        !run.stderr.contains("debugger") && !run.stderr.contains("bashdb"),
        "ambient marker must not attempt a debugger startup: {}",
        run.stderr
    );
}

#[test]
fn own_argv_debugger_still_attempts_startup() {
    // The strip removes only the AMBIENT copy; the shell's own
    // --debugger handling runs after import and must still engage (in
    // this environment the attempt reports the missing bashdb library,
    // exactly like the unmarked baseline).
    let run = spawn(&["--debugger", "-c", "echo RAN"], &[], b"");
    assert!(
        run.stderr.contains("debugger"),
        "own --debugger must still attempt startup: {}",
        run.stderr
    );
}

// ---------------------------------------------------------------- F8 ---

#[test]
fn ambient_comsub_body_does_not_rewrite_fatal_status() {
    let run = run_c(": ${UNDEF_X:?boom}", &[("__RUBASH_COMSUB_BODY", "1")]);
    assert_eq!(run.code, Some(127), "top-level fatal keeps status 127");
}

#[test]
fn control_fatal_status_is_127() {
    let run = run_c(": ${UNDEF_X:?boom}", &[]);
    assert_eq!(run.code, Some(127));
}

#[test]
fn genuine_comsub_fatal_stays_nonfatal_to_the_outer_shell() {
    // Inside a real command substitution the same expansion is a local
    // failure: the substitution yields empty, the shell continues.
    let run = run_c("x=$(: ${UNDEF_X:?boom}); echo \"AFTER rc=$?\"", &[]);
    assert_eq!(run.code, Some(0));
    assert!(run.stdout.contains("AFTER"), "{}", run.stdout);
}

// ---------------------------------------------------------------- F9 ---

#[test]
fn ambient_posix_marker_alone_engages_nothing() {
    assert_eq!(
        posix_option("set -o | grep '^posix'", &[("__RUBASH_POSIX_MODE", "1")]),
        "off"
    );
    let run = run_c("export 1x=2; echo ALIVE", &[("__RUBASH_POSIX_MODE", "1")]);
    assert_eq!(run.code, Some(0));
    assert!(run.stdout.contains("ALIVE"), "{}", run.stdout);
}

#[test]
fn ambient_setopt_posix_alone_engages_nothing() {
    assert_eq!(
        posix_option("set -o | grep '^posix'", &[("__RUBASH_SETOPT_posix", "1")]),
        "off"
    );
    let run = run_c("export 1x=2; echo ALIVE", &[("__RUBASH_SETOPT_posix", "1")]);
    assert_eq!(run.code, Some(0));
    assert!(run.stdout.contains("ALIVE"), "{}", run.stdout);
}

#[test]
fn split_pair_forgery_engages_nothing() {
    // Both halves planted, disagreeing in the direction the census
    // found most confusing (marker on, option key off-value absent):
    // the entry must still come up with the pair OFF and consistent.
    let run = run_c(
        "export 1x=2; echo ALIVE",
        &[("__RUBASH_POSIX_MODE", "1"), ("__RUBASH_SETOPT_posix", "0")],
    );
    assert_eq!(run.code, Some(0));
    assert!(run.stdout.contains("ALIVE"), "{}", run.stdout);
}

#[test]
fn posixly_correct_channel_still_engages_posix() {
    let run = run_c(
        "set -o | grep '^posix'; export 1x=2",
        &[("POSIXLY_CORRECT", "1")],
    );
    assert!(
        run.stdout
            .lines()
            .any(|line| line.starts_with("posix") && line.ends_with("on")),
        "{}",
        run.stdout
    );
    assert_eq!(run.code, Some(1), "posix dialect makes the export fatal");
}

#[test]
fn posix_argv_channel_still_engages_posix() {
    let run = spawn(&["--posix", "-c", "set -o | grep '^posix'"], &[], b"");
    assert!(
        run.stdout
            .lines()
            .any(|line| line.starts_with("posix") && line.ends_with("on")),
        "{}",
        run.stdout
    );
}

#[test]
fn in_session_posix_flip_still_engages_both_halves() {
    let run = run_c("set -o posix; export 1x=2", &[]);
    assert_eq!(run.code, Some(1), "in-session flip makes the export fatal");
}

#[test]
fn genuine_respawn_inherits_posix_consistently() {
    // A fork-shaped child of a posix parent keeps the pair together:
    // the process-substitution child reports the option on and applies
    // the dialect.
    let run = run_c("set -o posix; cat <(set -o | grep '^posix')", &[]);
    assert!(
        run.stdout
            .lines()
            .any(|line| line.starts_with("posix") && line.ends_with("on")),
        "{}",
        run.stdout
    );
    let run = run_c("set +o posix; cat <(set -o | grep '^posix')", &[]);
    assert!(
        run.stdout
            .lines()
            .any(|line| line.starts_with("posix") && line.ends_with("off")),
        "{}",
        run.stdout
    );
}
