//! rubash#1820: a prebuilt FIFO used with a background writer and a
//! foreground reader hung forever. Root cause: this port opens a
//! redirect target more than once — a throwaway probe/prepare open
//! (setup walk, compound prepare phase, input probe) plus the bind
//! open — while GNU opens it exactly once (redir.c
//! do_redirection_internal). A FIFO open has rendezvous semantics:
//! the blocking probe open consumed the pairing, returned and closed,
//! and the bind open then waited for a second peer that had already
//! finished its own open/write/close cycle.
//!
//! Oracle: GNU bash 5.3.20 passes both seed shapes (writer/reader
//! rendezvous through the single bind open on each side).
//!
//! The pins run each seed several times: the failure was a race
//! (roughly half of runs hung), so a single pass proves little. A
//! watchdog kills a hung run and fails the pin instead of stalling
//! the suite.

#![cfg(unix)]

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Local stand-in for the `tempfile` crate (not a dev-dependency of
/// this repo): a unique scratch directory removed on drop.
struct ScratchDir(std::path::PathBuf);

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

fn mkfifo(path: &std::path::Path) {
    let status = Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("spawn mkfifo");
    assert!(status.success(), "mkfifo {path:?}");
}

/// Drain one pipe on a thread, forwarding chunks as they arrive so
/// the harness never depends on every write end closing
/// (rubash#1868): a killed shell can leave an orphaned background
/// child holding the pipe open, and a blocking `read_to_end` after
/// the kill then stalls the whole suite forever.
fn drain_pipe<R: Read + Send + 'static>(mut pipe: R) -> std::sync::mpsc::Receiver<Vec<u8>> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        loop {
            match pipe.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if sender.send(chunk[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    receiver
}

/// Collect whatever a drain thread produced, giving up after
/// `grace` — the bytes are diagnostics for a run whose verdict is
/// already decided, so a stuck writer must not stall the harness.
fn collect_drained(receiver: std::sync::mpsc::Receiver<Vec<u8>>, grace: Duration) -> Vec<u8> {
    let mut bytes = Vec::new();
    let deadline = Instant::now() + grace;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match receiver.recv_timeout(remaining) {
            Ok(chunk) => bytes.extend_from_slice(&chunk),
            Err(_) => break,
        }
    }
    bytes
}

/// Run `case.sh` (written into a scratch dir that also holds the
/// prebuilt fifo `f`) under rubash with a cleared environment, with a
/// watchdog deadline. Returns (stdout, stderr, success); a hung run
/// is killed and reported as failure.
///
/// rubash#1868 anti-hang contract: the shell runs in its OWN process
/// group and the watchdog kills the whole group, so an orphaned
/// wrapper cannot survive the kill; the pipes drain on threads from
/// the start (a full pipe can never gate the shell) and collection
/// after a kill is grace-bounded, so this helper returns in bounded
/// time no matter what the run leaves behind.
fn run_script_watchdog(script: &str, timeout: Duration) -> (String, String, bool) {
    use std::os::unix::process::CommandExt;
    let dir = scratch_tempdir();
    mkfifo(&dir.0.join("f"));
    let script_path = dir.0.join("case.sh");
    std::fs::write(&script_path, script).expect("write script");
    let mut child = Command::new(rubash_bin())
        .arg(&script_path)
        .arg(dir.0.join("f"))
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", &dir.0)
        .current_dir(&dir.0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .expect("spawn rubash");
    let pgid = child.id();
    let stdout_rx = drain_pipe(child.stdout.take().expect("stdout pipe"));
    let stderr_rx = drain_pipe(child.stderr.take().expect("stderr pipe"));
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break Some(status),
            None => {
                if Instant::now() >= deadline {
                    // Kill the whole group, not just the shell: a
                    // background wrapper that outlives its shell keeps
                    // the inherited pipes (and possibly the FIFO
                    // rendezvous) hostage.
                    let _ = Command::new("/bin/kill")
                        .arg("-KILL")
                        .arg(format!("-{pgid}"))
                        .status();
                    let _ = child.kill();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    let _ = child.wait();
    // After a clean exit every writer is gone and the drains end at
    // once; after a kill, bound the wait so a writer outside the
    // group still cannot stall the suite (rubash#1868).
    let grace = if status.is_some() {
        Duration::from_secs(10)
    } else {
        Duration::from_secs(5)
    };
    let stdout = collect_drained(stdout_rx, grace);
    let stderr = collect_drained(stderr_rx, grace);
    (
        String::from_utf8_lossy(&stdout).into_owned(),
        String::from_utf8_lossy(&stderr).into_owned(),
        status.is_some_and(|s| s.success()),
    )
}

#[test]
fn issue1820_fifo_background_writer_foreground_read() {
    // Issue seed, case 1 (verbatim shape): immediate background
    // writer, simple foreground read.
    for _ in 0..5 {
        let (stdout, stderr, success) = run_script_watchdog(
            "fifo=\"$1\"\n( printf \"hello\\n\" > \"$fifo\" ) &\nread line < \"$fifo\"\necho \"got=$line\"\n",
            Duration::from_secs(20),
        );
        assert!(success, "run must complete, stderr: {stderr}");
        assert_eq!(stdout, "got=hello\n");
        assert!(stderr.is_empty(), "no diagnostics expected: {stderr}");
    }
}

#[test]
fn issue1820_fifo_delayed_writer_group_read() {
    // Issue seed, case 2 (verbatim shape): the writer sleeps before
    // opening; the foreground reads two lines through a brace group's
    // single input redirect.
    for _ in 0..5 {
        let (stdout, stderr, success) = run_script_watchdog(
            "fifo=\"$1\"\n( sleep 1; printf \"one\\ntwo\\n\" > \"$fifo\" ) &\n{ read a; read b; } < \"$fifo\"\necho \"a=$a b=$b\"\n",
            Duration::from_secs(20),
        );
        assert!(success, "run must complete, stderr: {stderr}");
        assert_eq!(stdout, "a=one b=two\n");
        assert!(stderr.is_empty(), "no diagnostics expected: {stderr}");
    }
}

#[test]
fn issue1820_fifo_unreadable_target_reports_permission_denied() {
    // The probe layer validates FIFO targets with access(2) instead
    // of an open; the error surface must stay GNU's: a redirection
    // error naming the target, status 1, no hang.
    // Root bypasses DAC entirely (the open would succeed and block on
    // the rendezvous), so the shape is untestable as uid 0.
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let uid0 = status
        .lines()
        .find(|line| line.starts_with("Uid:"))
        .and_then(|line| line.split_whitespace().nth(1))
        .is_some_and(|uid| uid == "0");
    if uid0 {
        return;
    }
    let dir = scratch_tempdir();
    let fifo = dir.0.join("f");
    mkfifo(&fifo);
    std::fs::set_permissions(&fifo, std::os::unix::fs::PermissionsExt::from_mode(0o000))
        .expect("chmod fifo");
    let script_path = dir.0.join("case.sh");
    std::fs::write(&script_path, "read line < \"$1\"\necho \"rc=$?\"\n").expect("write script");
    let output = Command::new(rubash_bin())
        .arg(&script_path)
        .arg(&fifo)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", &dir.0)
        .current_dir(&dir.0)
        .stdin(Stdio::null())
        .output()
        .expect("spawn rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(stdout, "rc=1\n", "read must fail with status 1: {stdout}");
    assert!(
        stderr.contains("Permission denied"),
        "GNU redirection error expected: {stderr}"
    );
}
