//! rubash#1583: an input process substitution `<(body)` must start
//! its body at EXPANSION time (GNU subst.c:6362 process_substitute
//! forks the child when the substitution is expanded), not lazily at
//! the first read of the resulting fd.
//!
//! Adjudication note (fix6/pipe-diag): on the wave-6 baseline
//! (f6b5a3f5) the eager start already happens — `bind_input_process_
//! substitution_child` (rubash#1021) spawns a real child at expansion
//! and the sync-collection path is only a fallback — so this file pins
//! the contract with deterministic discriminators instead of changing
//! code. The audit's original zero-delay probe (check a flag
//! immediately, with no wait at all) is a scheduling race that GNU
//! bash 5.3.20 itself loses on a loaded machine, so it is NOT pinned;
//! the discriminators below are: a side effect visible after a
//! non-consuming delay, and body completion after the parent exited
//! without ever reading.

#![cfg(unix)]

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Local stand-in for the `tempfile` crate (not a dev-dependency of
/// this repo): a unique scratch directory removed on drop.
struct ScratchDir(std::path::PathBuf);

impl ScratchDir {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch_tempdir() -> ScratchDir {
    let dir = std::env::temp_dir().join(format!(
        "rubash-scratch-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    ScratchDir(dir)
}

fn rubash_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_rubash"))
}

fn run_script(dir: &std::path::Path, script: &str) -> (String, String) {
    let script_path = dir.join("case.sh");
    std::fs::write(&script_path, script).expect("write script");
    let output = Command::new(rubash_bin())
        .arg(&script_path)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", dir)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn issue1583_procsub_body_starts_before_any_read() {
    let dir = scratch_tempdir();
    // The delay between expansion and the flag check is an external
    // `sleep` command — nothing reads fd 3, so a lazy implementation
    // (start on first read) has not started the body when the check
    // runs; GNU has.
    let (stdout, stderr) = run_script(
        dir.path(),
        "exec 3< <(touch started.flag; sleep 2; echo late)\n\
         /bin/sleep 0.5\n\
         if [[ -e started.flag ]]; then echo started=YES; else echo started=NO; fi\n\
         exec 3<&-\n",
    );
    assert_eq!(stdout, "started=YES\n", "stderr: {stderr:?}");
}

#[test]
fn issue1583_procsub_body_completes_after_parent_exit_without_read() {
    let dir = scratch_tempdir();
    let (stdout, stderr) = run_script(
        dir.path(),
        "exec 3< <(/bin/sleep 0.3; touch done.flag; echo x)\nexit 0\n",
    );
    assert_eq!(stdout, "", "stderr: {stderr:?}");
    // The detached body must run to completion after the shell is
    // gone (GNU's forked child is not killed by the parent's exit).
    let flag = dir.path().join("done.flag");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !flag.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        flag.exists(),
        "procsub body never completed after parent exit (done.flag missing)"
    );
}

#[test]
fn issue1583_procsub_input_still_readable_after_eager_start() {
    let dir = scratch_tempdir();
    // Control: eager start must not disturb the data path — the fd
    // still yields the body's output in order.
    let (stdout, stderr) = run_script(
        dir.path(),
        "exec 3< <(printf 'one\\ntwo\\n')\n\
         read -r a <&3\n\
         read -r b <&3\n\
         echo \"got=$a,$b\"\n\
         exec 3<&-\n",
    );
    assert_eq!(stdout, "got=one,two\n", "stderr: {stderr:?}");
}
