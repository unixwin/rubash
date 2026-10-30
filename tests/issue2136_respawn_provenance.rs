//! Issue rubash#2136 regressions: respawn provenance is established by
//! the process's actual parent, never by ambient environment strings.
//!
//! The defect: `__RUBASH_SHELL_PID` in a fresh process's environment was
//! adopted as the child's own `$$` (a value naming a process that may
//! not exist) and simultaneously opened the internal-respawn gate, after
//! which a forged `__RUBASH_POSITIONAL_PARAMS` seeded `$1..$n` and a
//! forged `__RUBASH_SUBSHELL_DEPTH` rewrote `$BASH_SUBSHELL`. The gate
//! itself was sound (the transport without a provenance marker was
//! ignored) — its key was the problem.
//!
//! The fix: every fork-shaped spawn site (async/subshell wrapper,
//! coproc, both process-substitution forms) also stamps
//! `__RUBASH_SPAWNER_PID` with its own process id, and `Executor::new`
//! honors the claim only when the stamp names the child's actual
//! parent (getppid), or the child was orphaned to the namespace init
//! and the stamped spawner is gone. GNU ground truth: a forked child's
//! `$$` stays the main shell's pid (variables.c), a subshell runs one
//! level deeper (execute_cmd.c:1576 execute_in_subshell,
//! subst.c:6362 process_substitute), and a plain nested shell reports
//! its own pid — none of which an unrelated parent may rewrite.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const WATCHDOG: Duration = Duration::from_secs(20);

fn bin() -> String {
    env!("CARGO_BIN_EXE_rubash").replace('\\', "/")
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i2136-{tag}-{}-{}",
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
    pid: u32,
}

/// Spawn the binary with the whole provenance family scrubbed unless the
/// probe re-adds it, and a watchdog deadline.
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
        .env_remove("__RUBASH_SHELL_PID")
        .env_remove("__RUBASH_COPROC_CHILD")
        .env_remove("__RUBASH_SPAWNER_PID")
        .env_remove("__RUBASH_SUBSHELL_DEPTH")
        .env_remove("__RUBASH_BG_DEPTH")
        .env_remove("__RUBASH_BG_BASHPID_BASE")
        .env_remove("__RUBASH_POSITIONAL_PARAMS")
        .stdin(Stdio::from(
            std::fs::File::open(&in_path).expect("stdin file"),
        ))
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (key, value) in extra_env {
        command.env(key, value);
    }
    // Hermetic fd table: the local build slot holds its flock on fd 9
    // and children inherit it; an extra inherited fd flips the shell's
    // async spawn-path gate (direct spawn vs wrapper respawn), which
    // changes the pid topology these pins assert on. CI never runs
    // under the slot, but the pin must mean the same thing locally.
    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        command.pre_exec(|| {
            libc::close(9);
            Ok(())
        });
    }
    let mut child = command.spawn().expect("spawn rubash");
    let pid = child.id();
    let deadline = Instant::now() + WATCHDOG;
    let code = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break Some(status.code().unwrap_or(-1)),
            None => {
                assert!(Instant::now() < deadline, "watchdog: child {pid} hung");
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
        pid,
    }
}

fn run_c(source: &str, extra_env: &[(&str, &str)]) -> Run {
    spawn(&["-c", source], extra_env, b"")
}

/// The positional transport's storage encoding (markers.rs), carrying
/// two forged parameters.
const FORGED_PARAMS: &str = "\u{1d}([0]=\"FORGED_A\" [1]=\"FORGED_B\")";

#[test]
fn forged_shell_pid_is_not_adopted_as_dollar_dollar() {
    let run = run_c("echo $$", &[("__RUBASH_SHELL_PID", "424242")]);
    assert_eq!(run.code, Some(0));
    assert_eq!(
        run.stdout,
        run.pid.to_string(),
        "ambient __RUBASH_SHELL_PID must not rewrite $$"
    );
}

#[test]
fn control_shell_pid_is_own_pid() {
    let run = run_c("echo $$", &[]);
    assert_eq!(run.stdout, run.pid.to_string());
}

#[test]
fn forged_positional_chain_stays_shut_without_valid_provenance() {
    let run = run_c(
        "echo COUNT=$# FIRST=$1 SECOND=$2",
        &[
            ("__RUBASH_SHELL_PID", "424242"),
            ("__RUBASH_POSITIONAL_PARAMS", FORGED_PARAMS),
        ],
    );
    assert_eq!(run.stdout, "COUNT=0 FIRST= SECOND=");
}

#[test]
fn spawner_stamp_naming_a_stranger_does_not_open_the_gate() {
    // A stamp is present but names a process that is not this child's
    // parent: the claim must still fail (the gate reads parentage, not
    // the mere presence of a second string).
    let mut dead = Command::new(bin())
        .args(["-c", "exit"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn short-lived rubash");
    let dead_pid = dead.id().to_string();
    let _ = dead.wait();
    let run = run_c(
        "echo COUNT=$# PID=$$",
        &[
            ("__RUBASH_SHELL_PID", "424242"),
            ("__RUBASH_SPAWNER_PID", &dead_pid),
            ("__RUBASH_POSITIONAL_PARAMS", FORGED_PARAMS),
        ],
    );
    assert_eq!(
        run.stdout,
        format!("COUNT=0 PID={}", run.pid),
        "a spawner stamp naming a non-parent must not validate the claim"
    );
}

#[test]
fn forged_subshell_depth_is_not_adopted() {
    let run = run_c(
        "echo DEPTH=$BASH_SUBSHELL",
        &[("__RUBASH_SUBSHELL_DEPTH", "7")],
    );
    assert_eq!(run.stdout, "DEPTH=0");
}

#[test]
fn genuine_async_child_keeps_shell_pid_and_depth() {
    // GNU execute_cmd.c:1576: the forked async child keeps the shell's
    // $$ and runs one subshell level deeper.
    let run = run_c(
        "echo \"outer=$$\"; ( echo \"inner=$$/$BASH_SUBSHELL\" ) & wait",
        &[],
    );
    let outer = run
        .stdout
        .lines()
        .find(|line| line.starts_with("outer="))
        .expect("outer line")
        .trim_start_matches("outer=");
    let inner = run
        .stdout
        .lines()
        .find(|line| line.starts_with("inner="))
        .expect("inner line")
        .trim_start_matches("inner=");
    assert_eq!(inner, format!("{outer}/1"));
}

#[test]
fn genuine_procsub_child_keeps_positionals_depth_and_shell_pid() {
    // GNU subst.c:6362: the process-substitution producer is a forked
    // child — it inherits the positional window, keeps $$, and runs one
    // subshell level deeper.
    let run = run_c(
        "set -- P1 P2; echo \"outer=$$\"; cat <(printf 'in=%s/%s/%s' \"$1\" \"$BASH_SUBSHELL\" \"$$\")",
        &[],
    );
    let outer = run
        .stdout
        .lines()
        .find(|line| line.starts_with("outer="))
        .expect("outer line")
        .trim_start_matches("outer=");
    assert!(
        run.stdout.contains(&format!("in=P1/1/{outer}")),
        "procsub child must inherit positionals, depth and $$: {}",
        run.stdout
    );
}

#[test]
fn genuine_coproc_child_still_runs() {
    let run = run_c(
        "coproc { echo hi; }; read -u ${COPROC[0]} x; echo \"coproc:$x\"",
        &[],
    );
    assert_eq!(run.stdout, "coproc:hi");
}

#[test]
fn orphaned_async_child_still_adopts_provenance() {
    // The shell exits immediately after spawning the async child; the
    // child is orphaned (to init) before or while it starts. GNU's
    // forked child keeps the shell's $$ and depth regardless — the
    // orphan arm of the provenance check covers the race.
    let dir = scratch("orphan");
    let out = dir.join("out");
    let source = format!(
        "( printf '%s/%s' \"$$\" \"$BASH_SUBSHELL\" > \"{}\" ) & exit",
        out.to_string_lossy().replace('\\', "/")
    );
    let parent = run_c(&source, &[]);
    let deadline = Instant::now() + WATCHDOG;
    let content = loop {
        if let Ok(content) = std::fs::read_to_string(&out) {
            if !content.is_empty() {
                break content;
            }
        }
        assert!(Instant::now() < deadline, "orphan child never wrote");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(content, format!("{}/1", parent.pid));
}

#[test]
fn plain_nested_shell_reports_its_own_pid() {
    // A rubash launched as an ordinary external command is a top-level
    // shell: no provenance crosses (control for the mirror hygiene), so
    // the nested shell's $$ is its own process pid — the job pid the
    // outer shell recorded for it.
    let source = format!("\"{}\" -c 'echo $$' & echo \"job=$!\"; wait", bin());
    let run = run_c(&source, &[]);
    let inner = run
        .stdout
        .lines()
        .find(|line| !line.is_empty() && line.bytes().all(|b| b.is_ascii_digit()))
        .expect("inner pid line");
    let job = run
        .stdout
        .lines()
        .find(|line| line.starts_with("job="))
        .expect("job line")
        .trim_start_matches("job=");
    assert_eq!(inner, job, "nested shell must report its own pid");
}

fn _unused(_: &Path) {}
