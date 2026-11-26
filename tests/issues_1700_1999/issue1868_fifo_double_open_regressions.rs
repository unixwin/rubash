//! Regression pins for rubash#1868 (the load-dependent hang of the
//! #1820 fifo pins).
//!
//! Root cause, established by strace: a builtin's `> fifo` redirect was
//! physically opened TWICE — a create/truncate validation open whose
//! File was dropped (ordered output walk / compound bind walk), then
//! the buffered-output flush re-open. On a FIFO the dropped open
//! consumes the rendezvous pairing and closes without writing: a
//! reader whose first read starts in the zero-writer gap before the
//! flush open takes a spurious EOF (its shell exits 0 with an empty
//! line), and the flush open then blocks forever with no reader left.
//! GNU redir.c do_redirections performs ONE open per redirection. The
//! fix access-probes FIFO targets at validation time (the #1820 probe
//! carve-out) so the flush/bind open is the single pairing open, and
//! an empty stream still performs its pairing open at the flush site
//! (`printf "" > fifo` blocks for a reader like GNU).
//!
//! The harness is the #1868-hardened watchdog: the child runs in its
//! own process group, stdout/stderr drain on threads from the start,
//! a timeout kills the whole group, and post-exit collection is
//! grace-bounded — an orphaned process holding the pipes can never
//! stall the test the way the pre-#1868 harness stalled.
#![cfg(unix)]

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

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

fn run_script_watchdog(
    script: &str,
    timeout: Duration,
) -> (std::process::ExitStatus, String, String) {
    use std::os::unix::process::CommandExt;
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1868-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    fs::create_dir_all(&dir).expect("create temp dir");
    let script_path: PathBuf = dir.join("case.sh");
    fs::write(&script_path, script).expect("write script");
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_rubash"));
    let mut child = Command::new(bin)
        .arg(&script_path)
        .current_dir(&dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .expect("spawn rubash");
    let stdout_rx = drain_pipe(child.stdout.take().expect("stdout pipe"));
    let stderr_rx = drain_pipe(child.stderr.take().expect("stderr pipe"));
    let pgid = child.id();
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait().expect("poll child") {
            Some(status) => break Some(status),
            None => {
                if Instant::now() >= deadline {
                    let _ = Command::new("/bin/kill")
                        .arg("-KILL")
                        .arg(format!("-{pgid}"))
                        .status();
                    let _ = child.kill();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    };
    let status = match status {
        Some(status) => status,
        None => {
            let _ = child.wait();
            let stdout = collect_drained(stdout_rx, Duration::from_secs(2));
            let stderr = collect_drained(stderr_rx, Duration::from_secs(1));
            let _ = fs::remove_dir_all(&dir);
            panic!(
                "watchdog timeout after {timeout:?} (the #1868 hang): \
                 partial stdout={:?} stderr={:?}",
                String::from_utf8_lossy(&stdout),
                String::from_utf8_lossy(&stderr)
            );
        }
    };
    let stdout = collect_drained(stdout_rx, Duration::from_secs(2));
    let stderr = collect_drained(stderr_rx, Duration::from_secs(1));
    let _ = fs::remove_dir_all(&dir);
    (
        status,
        String::from_utf8_lossy(&stdout).into_owned(),
        String::from_utf8_lossy(&stderr).into_owned(),
    )
}

fn run_fifo_shape(body: &str) -> (std::process::ExitStatus, String, String) {
    let script = format!("fifo=\"$PWD/f\"\nmkfifo \"$fifo\"\n{body}\n");
    run_script_watchdog(&script, Duration::from_secs(20))
}

/// The #1820 seed shape, looped: the pre-fix failure was racy
/// (~5% per run under load), so a single pass proves little; the
/// loop plus the deterministic `empty` pin below bracket it.
#[test]
fn issue1868_seed_shape_repeated_no_eof_race() {
    for _ in 0..8 {
        let (status, stdout, stderr) = run_fifo_shape(
            "( printf \"hello\\n\" > \"$fifo\" ) &\nread line < \"$fifo\"\necho \"got=$line\"",
        );
        assert!(status.success(), "stderr: {stderr}");
        assert_eq!(stdout, "got=hello\n", "stderr: {stderr}");
    }
}

/// Deterministic pre-fix hang: an empty-output builtin performed NO
/// fifo open at all once the validation open was carved out — the
/// pairing open must still happen (GNU opens at redirection time).
#[test]
fn issue1868_empty_output_builtin_still_pairs_fifo_open() {
    let (status, stdout, stderr) =
        run_fifo_shape("printf \"\" > \"$fifo\" &\nread line < \"$fifo\"\necho \"got=[${line}]\"");
    assert!(status.success(), "stderr: {stderr}");
    assert_eq!(stdout, "got=[]\n", "stderr: {stderr}");
}

#[test]
fn issue1868_compound_and_append_writers_pair_once() {
    let (status, stdout, stderr) = run_fifo_shape(
        "{ printf \"hello\\n\"; } > \"$fifo\" &\nread line < \"$fifo\"\necho \"got=$line\"",
    );
    assert!(status.success(), "stderr: {stderr}");
    assert_eq!(stdout, "got=hello\n", "stderr: {stderr}");

    let (status, stdout, stderr) = run_fifo_shape(
        "( printf \"hello\\n\" >> \"$fifo\" ) &\nread line < \"$fifo\"\necho \"got=$line\"",
    );
    assert!(status.success(), "stderr: {stderr}");
    assert_eq!(stdout, "got=hello\n", "stderr: {stderr}");
}

/// The carve-out must not disturb plain-file semantics: the file is
/// still created/truncated by the redirect even when a later write
/// re-opens it.
#[test]
fn issue1868_plain_file_redirect_semantics_unchanged() {
    let script = "printf \"alpha\\n\" > out\nprintf \"beta\\n\" > out\ncat out\n\
                  : > empty\nwc -c < empty\n";
    let (status, stdout, stderr) = run_script_watchdog(script, Duration::from_secs(20));
    assert!(status.success(), "stderr: {stderr}");
    assert_eq!(stdout, "beta\n0\n", "stderr: {stderr}");
}
