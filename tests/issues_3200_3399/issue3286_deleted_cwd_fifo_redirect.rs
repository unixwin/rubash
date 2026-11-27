//! rubash#3286 — bare-`exec` output/append redirect to an EXISTING
//! FIFO from a deleted session cwd (absorb of PR #3363's exec face;
//! the command-attached faces live in
//! tests/issue3286_deleted_cwd_fifo_redirect_regressions.rs, landed
//! by #3735).
//!
//! GNU (redir.c:237 `do_redirections` -> redir.c:790
//! `do_redirection_internal` -> redir.c:725 `redir_open`): a redirect
//! target is opened exactly ONCE, in the child, by a plain
//! `open(filename, flags, mode)` — for `>` that is
//! O_WRONLY|O_CREAT|O_TRUNC, for `>>` O_WRONLY|O_CREAT|O_APPEND.
//! The child inherits the deleted cwd as an inode, so a relative
//! operand resolves through it; an O_WRONLY open of a FIFO blocks
//! until a reader opens — the rendezvous — and then succeeds.
//! There is no probe stage in GNU.
//!
//! rubash's redirect machine has a probe/bind split: the probe
//! classifies an existing FIFO by anchor fstatat/faccessat (no fd
//! taken, rubash#1820) and returns no File for it, so the bind
//! performs the one real open. The two exec faces pinned here are
//! the ones #3735 did not cover: the persistent bind
//! (`exec > fifo`, `set_fd_output_file`) and the replacement spawn
//! (`exec <cmd> >> fifo`, `execute_exec_spawn` ->
//! `exec_spawn_open_output`). Both opens go through the cwd anchor
//! for a relative operand in a deleted cwd.
//!
//! Pins assert COMPLETION FORMS (output bytes + exit status), never
//! timing. Unix only: the cwd anchor is a Unix directory handle.
//!
//! Oracle: GNU bash 5.3.20, captain-measured (issue body, three-party
//! probe log on the dispatch tree): reader present,
//! `echo hello > ../fifo` -> wrc=0, reader gets hello (blocking
//! rendezvous, then success); no reader -> blocks (standard FIFO
//! write-open semantics). Live cwd has no divergence — the string
//! open itself completes the rendezvous.

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

/// Poll `../got` until the reader delivers the bytes (the reader only
/// flushes `got` when it sees EOF at shell exit).
fn expect_reader_got(dir: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(landed) = std::fs::read_to_string(dir.join("got")) {
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
    expect_reader_got(&dir.0);
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
    expect_reader_got(&dir.0);
}

#[test]
fn issue3286_exec_command_append_redirect_to_existing_fifo_from_deleted_cwd() {
    // Replacement-spawn twin: `exec <cmd> >> fifo` replaces the
    // shell image, so the append open runs in the spawn walk
    // (trap_exec.rs execute_exec_spawn -> exec_spawn_open_output),
    // not the persistent bind. Same verdict shape as the bare-exec
    // pins: the reader gets `hello`, the harness sees nothing.
    let dir = scratch("exec-cmd-append");
    let script = format!(
        "{}( timeout 10 /usr/bin/cat ../fifo > ../got ) &\n\
         sleep 1\n\
         exec echo hello >> ../fifo\n",
        prelude()
    );
    let (stdout, stderr, code) = run_rubash(&dir.0, &script, Duration::from_secs(30));
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(
        stdout, "",
        "hello must go to the fifo, not the harness: {stderr}"
    );
    assert_eq!(stderr, "", "stderr:\n{stderr}");
    expect_reader_got(&dir.0);
}
