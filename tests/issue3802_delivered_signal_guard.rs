//! Regression pins for the rubash#3802 delivered-signal decode guard.
//!
//! GNU `jobs.c:1969-2015 printable_job_status()` decides the jobs
//! wording from the WAIT STATUS: only the `WIFSIGNALED` arm calls
//! `j_strsignal (WTERMSIG (p->status))` (`Killed`, `Terminated`, ...);
//! the `WIFEXITED` arm always renders `Exit %d`. Linux `wait` gives
//! rubash that status directly (`exit_status_signal`), but Windows
//! `ExitStatus` carries only the flattened code, so #3805 recorded the
//! signal this shell's own `kill` delivered and decoded it at
//! completion. That decode must stay conditioned on the reaped status
//! being exactly `128+signal`: a process that ignored or trapped the
//! delivered signal and later exited on its own is `WIFEXITED`, so
//! GNU renders its real status (`Exit 5`, `Exit 143`), never
//! `Terminated`.
//!
//! The trap legs are unix-only (Windows has no in-child signal
//! disposition: `kill` ends the process with `TerminateProcess
//! (128+signal)`, so those shapes cannot be constructed there). The
//! cross-platform legs keep the #3805 behaviour pinned: a real
//! SIGKILL/SIGTERM death still renders `Killed`/`Terminated`, and a
//! plain nonzero exit stays `Exit <code>`.

use std::path::PathBuf;
use std::process::Command;

fn rubash_in_dir(script: &str, tag: &str) -> (PathBuf, String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-3802g-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .current_dir(&dir)
        .output()
        .expect("spawn rubash");
    (
        dir.clone(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

fn read(dir: &PathBuf, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).unwrap_or_default()
}

fn jobs_and_wait(script: &str, tag: &str) -> (String, String) {
    let (dir, _out, _err, _code) = rubash_in_dir(script, tag);
    let jobs = read(&dir, "jobs.out");
    let wrc = read(&dir, "wrc.out");
    let _ = std::fs::remove_dir_all(&dir);
    (jobs, wrc)
}

/// GNU oracle (WSL bash 5.3.20): `[1]+  Exit 5   ...` / `wrc=5`.
#[cfg(unix)]
#[test]
fn issue3802_ignored_term_then_exit_5_is_exit_5() {
    let (jobs, wrc) = jobs_and_wait(
        "\"$BASH\" -c 'trap \"\" TERM; sleep 0.6; exit 5' & p=$!; \
         sleep 0.2; kill -TERM $p; sleep 0.9; jobs > jobs.out; \
         wait $p; echo wrc=$? > wrc.out",
        "ignoreterm",
    );
    assert!(
        jobs.contains("Exit 5"),
        "a job that ignored SIGTERM and exited 5 must render Exit 5, jobs.out was {jobs:?}"
    );
    assert!(
        !jobs.contains("Terminated"),
        "a WIFEXITED death after a delivered SIGTERM must not render Terminated, jobs.out was {jobs:?}"
    );
    assert!(wrc.contains("wrc=5"), "wait reported {wrc:?}");
}

/// GNU oracle (WSL bash 5.3.20): `[1]+  Exit 143   ...` / `wrc=143`.
#[cfg(unix)]
#[test]
fn issue3802_trapped_term_then_exit_143_is_exit_143() {
    let (jobs, wrc) = jobs_and_wait(
        "\"$BASH\" -c 'trap \"exit 143\" TERM; sleep 0.4; exit 0' & p=$!; \
         sleep 0.2; kill -TERM $p; sleep 0.9; jobs > jobs.out; \
         wait $p; echo wrc=$? > wrc.out",
        "trapterm",
    );
    assert!(
        jobs.contains("Exit 143"),
        "a trapped SIGTERM that exits 143 must render Exit 143, jobs.out was {jobs:?}"
    );
    assert!(
        !jobs.contains("Terminated"),
        "WIFEXITED 143 is not a signal death, jobs.out was {jobs:?}"
    );
    assert!(wrc.contains("wrc=143"), "wait reported {wrc:?}");
}

/// The guard must not disable the #3802 wording: a real SIGKILL death
/// (status 137 == 128+9) still renders `Killed`.
#[test]
fn issue3802_real_kill9_still_renders_killed() {
    let (jobs, wrc) = jobs_and_wait(
        "sleep 30 & p=$!; kill -9 $p; sleep 0.2; jobs > jobs.out; \
         wait $p; echo wrc=$? > wrc.out",
        "guardk9",
    );
    assert!(
        jobs.contains("Killed"),
        "SIGKILL death must render Killed, jobs.out was {jobs:?}"
    );
    assert!(wrc.contains("wrc=137"), "wait reported {wrc:?}");
}

/// Plain nonzero exit, no kill delivered: `Exit 7`, on every platform.
#[test]
fn issue3802_plain_exit_7_is_exit_7() {
    let (jobs, wrc) = jobs_and_wait(
        "\"$BASH\" -c 'sleep 0.1; exit 7' & p=$!; \
         sleep 0.6; jobs > jobs.out; wait $p; echo wrc=$? > wrc.out",
        "exit7",
    );
    assert!(
        jobs.contains("Exit 7"),
        "a plain exit 7 must render Exit 7, jobs.out was {jobs:?}"
    );
    assert!(wrc.contains("wrc=7"), "wait reported {wrc:?}");
}

/// `wait`-first shape: GNU prints no jobs row after the waited job is
/// retired (jobs.c wait_for marks it notified), and the status is still
/// 137. The row must not come back with a stale `Exit 137`/`Killed`.
#[test]
fn issue3802_wait_then_jobs_has_no_stale_row() {
    let (jobs, wrc) = jobs_and_wait(
        "sleep 30 & p=$!; kill -9 $p; wait $p; echo wrc=$? > wrc.out; \
         jobs > jobs.out",
        "waitfirst",
    );
    assert!(
        jobs.trim().is_empty(),
        "no job row after the waited death, jobs.out was {jobs:?}"
    );
    assert!(wrc.contains("wrc=137"), "wait reported {wrc:?}");
}
