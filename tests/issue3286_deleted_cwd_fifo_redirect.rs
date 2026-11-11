//! rubash#3286 — output/append redirect to an EXISTING FIFO from a
//! deleted session cwd.
//!
//! GNU (redir.c:237 `do_redirections` -> `do_redirection_internal`,
//! redir.c:725 `redir_open`): a redirect target is opened exactly
//! ONCE, in the child, by a plain `open(filename, flags, mode)` —
//! for `>` that is O_WRONLY|O_CREAT|O_TRUNC, for `>>`
//! O_WRONLY|O_CREAT|O_APPEND. The child inherits the deleted cwd
//! as an inode, so a relative operand resolves through it; an
//! O_WRONLY open of a FIFO blocks until a reader opens — the
//! rendezvous — and then succeeds. There is no probe stage in GNU.
//!
//! rubash's redirect machine has a probe/bind split, and the anchor
//! faces classified an existing FIFO OUT of the anchor open
//! (shell_options.rs `create_redirect_output` / `open_append_target`):
//! the probe classifies FIFOs by joined-path metadata — which a dead
//! cwd defeats — and the bind then fell back to a string-path
//! `File::create`/`open_redirect_append` that dies at the dead
//! component with instant ENOENT. The fix is the two-stage split
//! the issue prescribes: the probe classifies by anchor fstatat and
//! validates by anchor faccessat (no fd taken), the bind performs
//! the anchor O_WRONLY openat — GNU's single open.
//!
//! Pins assert COMPLETION FORMS (output bytes + exit status), never
//! timing, except the no-reader form, which asserts the BLOCKING
//! form (still running after a grace period — the issue prescribes
//! "timeout kill, don't assert an instant error"). Unix only: the
//! cwd anchor is a Unix directory handle.
//!
//! Oracle: GNU bash 5.3.20, captain-measured (issue body, three-party
//! probe log on the dispatch tree): reader present,
//! `echo hello > ../fifo` -> wrc=0, reader gets hello (blocking
//! rendezvous, then success); no reader -> blocks (standard FIFO
//! write-open semantics). Live cwd has no divergence — the string
//! open itself completes the rendezvous — so a live-cwd control pins
//! the choreography, not the fix.

#![cfg(unix)]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct ScratchDir(PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A canonicalized scratch root D: D/a is the directory the shell
/// enters and loses, D/fifo is the prebuilt FIFO in the parent.
fn scratch(case: &str) -> ScratchDir {
    let dir = std::env::temp_dir().join(format!("rubash-3286-{case}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("a")).expect("create a");
    let status = Command::new("mkfifo")
        .arg(dir.join("fifo"))
        .status()
        .expect("spawn mkfifo");
    assert!(status.success(), "mkfifo");
    let canonical = std::fs::canonicalize(&dir).expect("canonicalize scratch");
    ScratchDir(canonical)
}

fn rubash_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rubash"))
}

/// The deleted-cwd prelude shared by every pin: enter D/a, have a
/// subshell remove it, stay there.
fn prelude() -> &'static str {
    "cd a || exit 99\n(cd ..; rmdir a) || exit 98\n"
}

/// Spawn rubash on `script` (via -c) inside `dir`, with a watchdog
/// that kills the whole process group on timeout. Returns
/// (stdout, stderr, exit code or None when killed by the watchdog).
fn run_rubash(dir: &Path, script: &str, timeout: Duration) -> (String, String, Option<i32>) {
    use std::os::unix::process::CommandExt;
    let mut child = Command::new(rubash_bin())
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .expect("spawn rubash");
    let pgid = child.id();
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait().expect("poll rubash") {
            Some(status) => break Some(status),
            None if Instant::now() >= deadline => {
                let _ = Command::new("/bin/kill")
                    .arg("-KILL")
                    .arg(format!("-{pgid}"))
                    .status();
                let _ = child.kill();
                break None;
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    let _ = child.wait();
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    (stdout, stderr, status.and_then(|s| s.code()))
}

#[test]
fn issue3286_output_redirect_to_existing_fifo_from_deleted_cwd() {
    // Issue seed form: reader present, `echo hello > ../fifo`.
    // GNU: wrc=0, reader gets hello. The reader runs under a 10s
    // `timeout` so a failing writer cannot hang the suite on `wait`:
    // on the unrepaired tree the writer fails instantly and the
    // reader times out, bounding the run.
    let dir = scratch("out");
    let script = format!(
        "{}( timeout 10 /usr/bin/cat ../fifo > ../got ) &\n\
         echo hello > ../fifo; echo rc=$?\n\
         wait\n\
         /usr/bin/cat ../got; echo crc=$?\n",
        prelude()
    );
    let (stdout, stderr, code) = run_rubash(&dir.0, &script, Duration::from_secs(30));
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=0\nhello\ncrc=0\n", "stderr:\n{stderr}");
    let landed = std::fs::read_to_string(dir.0.join("got")).expect("reader got the bytes");
    assert_eq!(landed, "hello\n");
}

#[test]
fn issue3286_append_redirect_to_existing_fifo_from_deleted_cwd() {
    // Append twin of the seed form: `echo hello >> ../fifo`.
    let dir = scratch("append");
    let script = format!(
        "{}( timeout 10 /usr/bin/cat ../fifo > ../got ) &\n\
         echo hello >> ../fifo; echo rc=$?\n\
         wait\n\
         /usr/bin/cat ../got; echo crc=$?\n",
        prelude()
    );
    let (stdout, stderr, code) = run_rubash(&dir.0, &script, Duration::from_secs(30));
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=0\nhello\ncrc=0\n", "stderr:\n{stderr}");
    let landed = std::fs::read_to_string(dir.0.join("got")).expect("reader got the bytes");
    assert_eq!(landed, "hello\n");
}

#[test]
fn issue3286_output_redirect_to_fifo_without_reader_blocks() {
    // No-reader form: GNU blocks (standard FIFO write-open
    // semantics). Assert the BLOCKING form: still running after a
    // 3s grace period, and no instant ENOENT/rc=1 exit. The
    // watchdog kills the whole process group afterwards.
    let dir = scratch("noreader");
    let script = format!("{}echo hello > ../fifo; echo rc=$?\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script, Duration::from_secs(3));
    assert_eq!(
        code, None,
        "must still be blocked on the rendezvous, not exited: code={code:?} stdout={stdout:?} stderr={stderr:?}"
    );
}

#[test]
fn issue3286_output_redirect_to_fifo_from_live_cwd_control() {
    // Control: live cwd has no divergence — the string open itself
    // completes the rendezvous. Guards the choreography, not the fix.
    let dir = scratch("live");
    let script = "( timeout 10 /usr/bin/cat fifo > got ) &\n\
                  echo hello > fifo; echo rc=$?\n\
                  wait\n\
                  /usr/bin/cat got; echo crc=$?\n";
    let (stdout, stderr, code) = run_rubash(&dir.0, script, Duration::from_secs(30));
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=0\nhello\ncrc=0\n", "stderr:\n{stderr}");
}

#[test]
fn issue3286_exec_output_redirect_to_existing_fifo_from_deleted_cwd() {
    // Bare-`exec` twin of the seed form: the redirect binds the
    // shell's own fd 1 persistently (trap_exec.rs
    // execute_stdio_only_exec_redirect -> set_fd_output_file), so
    // the script itself can no longer report — the verdict is read
    // off the reader's file after the shell exits, polling briefly
    // because the reader only flushes `got` when it sees EOF at
    // shell exit. A failed exec leaves `hello` on the harness
    // stdout and the ENOENT on stderr instead.
    let dir = scratch("exec-out");
    let script = format!(
        "{}( timeout 10 /usr/bin/cat ../fifo > ../got ) &\n\
         sleep 1\n\
         exec > ../fifo\n\
         echo hello\n\
         sleep 1\n",
        prelude()
    );
    let (stdout, stderr, code) = run_rubash(&dir.0, &script, Duration::from_secs(30));
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(
        stdout, "",
        "hello must go to the fifo, not the harness: {stderr}"
    );
    assert_eq!(stderr, "", "stderr:\n{stderr}");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(landed) = std::fs::read_to_string(dir.0.join("got")) {
            assert_eq!(landed, "hello\n");
            return;
        }
        if Instant::now() >= deadline {
            panic!("reader never delivered hello to ../got");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn issue3286_exec_append_redirect_to_existing_fifo_from_deleted_cwd() {
    // Append twin of the bare-`exec` form above.
    let dir = scratch("exec-append");
    let script = format!(
        "{}( timeout 10 /usr/bin/cat ../fifo > ../got ) &\n\
         sleep 1\n\
         exec >> ../fifo\n\
         echo hello\n\
         sleep 1\n",
        prelude()
    );
    let (stdout, stderr, code) = run_rubash(&dir.0, &script, Duration::from_secs(30));
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(
        stdout, "",
        "hello must go to the fifo, not the harness: {stderr}"
    );
    assert_eq!(stderr, "", "stderr:\n{stderr}");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(landed) = std::fs::read_to_string(dir.0.join("got")) {
            assert_eq!(landed, "hello\n");
            return;
        }
        if Instant::now() >= deadline {
            panic!("reader never delivered hello to ../got");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn issue3286_missing_target_from_deleted_cwd_still_fails() {
    // Control: the fix must not manufacture success for a name the
    // inode lacks — GNU fails the open in the child with the target
    // as spelled. Note `> ../nonexistent` would CREATE the file in
    // the parent (O_CREAT through the deleted cwd's `..` — the same
    // face #3268 pins for `> ../g`); the missing face for an output
    // redirect is a missing PARENT component.
    let dir = scratch("missing");
    let script = format!("{}echo hello > ../nodir/x; echo rc=$?\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script, Duration::from_secs(30));
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr:\n{stderr}");
    assert!(
        stderr.contains("../nodir/x: No such file or directory"),
        "stderr:\n{stderr}"
    );
}
