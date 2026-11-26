//! Regression pins for rubash#3802 (jobs table wording for a
//! signal-killed background job on Windows).
//!
//! GNU jobs.c:1969-2015 `printable_job_status` renders a job that died
//! by a signal with the signal description — `j_strsignal (WTERMSIG
//! (p->status))`, i.e. `Killed` for SIGKILL, `Terminated` for SIGTERM
//! — and only a plain exit with a nonzero code as `Exit %d`. The
//! distinction is the wait status's WIFSIGNALED bit, which Windows
//! `ExitStatus` does not carry: rubash's own `kill` encodes the kill
//! as `TerminateProcess (128 + signal)` (builtins/kill.rs
//! `signal_process`), so a SIGKILL'd job reaps with exit code 137 and
//! no recorded signal, and the table rendered `[1]+ Exit 137 sleep 30`.
//! The fix records the signal this shell's own `kill` delivered and
//! decodes the reap against it, so the table renders `Killed`.
//!
//! Seed: the #3637 seed (`seed_r118a.sh`) checks 2-3 count `Killed`
//! in the output of
//!   sleep 30 & p=$!; kill -9 $p; sleep 0.2; jobs; wait $p
//! and read 0 on Windows (X299 evidence in #3802). The behaviour
//! (exactly one death report — the table row) was already correct;
//! only the wording differed.
//!
//! The `exit 137` control pins the other side of the distinction: a
//! job that exits normally with code 137 must keep rendering
//! `Exit 137`, not `Killed` — the signal is attributed only when
//! this shell's own `kill` delivered it, never from the exit code
//! alone.
//!
//! NOTE (environment): the KILL/TERM legs fail-first on Windows only.
//! On Linux the wait status already carries the signal
//! (`exit_status_signal`), so those legs pass before the fix there;
//! the `exit 137` control is the discriminating leg on both
//! platforms. Windows verification runs on X299 (rubash.exe) via
//! native `cargo test`; `scrubbed_cargo` does not apply there.

use std::path::PathBuf;
use std::process::Command;

fn rubash_in_dir(script: &str, tag: &str) -> (PathBuf, String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-3802-{tag}-{}", std::process::id()));
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

#[test]
fn issue3802_jobs_kill9_renders_killed() {
    let (dir, _out, _err, _code) = rubash_in_dir(
        "sleep 30 & p=$!; kill -9 $p; sleep 0.2; jobs > jobs.out; wait $p; echo wrc=$? > wrc.out",
        "k9",
    );
    let jobs = read(&dir, "jobs.out");
    let wrc = read(&dir, "wrc.out");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        jobs.contains("Killed"),
        "jobs must render a SIGKILL'd job as Killed, jobs.out was {jobs:?}"
    );
    assert!(
        jobs.contains("sleep 30"),
        "jobs row must name the command, jobs.out was {jobs:?}"
    );
    assert!(
        !jobs.contains("Exit 137"),
        "jobs must not render a SIGKILL'd job as Exit 137, jobs.out was {jobs:?}"
    );
    assert!(
        wrc.contains("wrc=137"),
        "wait must still report 137, wrc.out was {wrc:?}"
    );
}

#[test]
fn issue3802_jobs_kill_kill_renders_killed() {
    let (dir, _out, _err, _code) = rubash_in_dir(
        "sleep 30 & p=$!; kill -KILL $p; sleep 0.2; jobs > jobs.out; echo done; wait $p; echo wrc=$? > wrc.out",
        "kkill",
    );
    let jobs = read(&dir, "jobs.out");
    let wrc = read(&dir, "wrc.out");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        jobs.contains("Killed"),
        "jobs must render a SIGKILL'd job as Killed, jobs.out was {jobs:?}"
    );
    assert!(
        !jobs.contains("Exit 137"),
        "jobs must not render a SIGKILL'd job as Exit 137, jobs.out was {jobs:?}"
    );
    assert!(
        wrc.contains("wrc=137"),
        "wait must still report 137, wrc.out was {wrc:?}"
    );
}

#[test]
fn issue3802_jobs_kill_term_renders_terminated() {
    let (dir, _out, _err, _code) = rubash_in_dir(
        "sleep 30 & p=$!; kill -TERM $p; sleep 0.2; jobs > jobs.out; wait $p; echo wrc=$? > wrc.out",
        "kterm",
    );
    let jobs = read(&dir, "jobs.out");
    let wrc = read(&dir, "wrc.out");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        jobs.contains("Terminated"),
        "jobs must render a SIGTERM'd job as Terminated, jobs.out was {jobs:?}"
    );
    assert!(
        !jobs.contains("Exit 143"),
        "jobs must not render a SIGTERM'd job as Exit 143, jobs.out was {jobs:?}"
    );
    assert!(
        wrc.contains("wrc=143"),
        "wait must still report 143, wrc.out was {wrc:?}"
    );
}

#[test]
fn issue3802_jobs_plain_exit_137_stays_exit_137() {
    let (dir, _out, _err, _code) = rubash_in_dir(
        "\"$BASH\" -c 'exit 137' & sleep 0.5; jobs > jobs.out; echo done",
        "exit137",
    );
    let jobs = read(&dir, "jobs.out");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        jobs.contains("Exit 137"),
        "a job that exits normally with 137 must render Exit 137, jobs.out was {jobs:?}"
    );
    assert!(
        !jobs.contains("Killed"),
        "a plain exit 137 must not be misread as a signal death, jobs.out was {jobs:?}"
    );
}
