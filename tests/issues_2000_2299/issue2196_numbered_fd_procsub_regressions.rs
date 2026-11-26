//! Issue unixwin/rubash#2196: numbered-fd output process substitution
//!
//! Spawn-time fd state is decided where the child is forked:
//! GNU forks procsub consumers mid-do_redirections (redir.c:237,
//! subst.c:6362) so a consumer inherits the fd table as the
//! redirect entries BEFORE its own left it, and a coproc child
//! applies its construct-level redirects of any fd in the child
//! (execute_cmd.c:2477/4244). These pins run rubash as a
//! subprocess and observe files/stdout only; they were run by
//! executing the test binaries directly (outside the build
//! slot, whose flock fd 9 would leak into the children).

//! Verified against GNU Bash 5.3.20 (script-file probes in the
//! cluster workdir): see REPORT.md for the probe ledger.
//!
//! Harness note: these pins run the shell as a subprocess and
//! assert on FILES the detached consumers write, polled to a
//! deadline — the consumers are async children (subst.c FORK_ASYNC)
//! whose writes race the shell's exit by design. No pin depends on
//! fd 9 or on the test process's own fd table shape.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-fd-family-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
    dir
}

fn rubash_stdin(script: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash stdin");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(script.as_bytes())
        .expect("write stdin");
    drop(child.stdin.take());
    let mut out = child.stdout.take().expect("stdout pipe");
    let mut err = child.stderr.take().expect("stderr pipe");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut out, &mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut err, &mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    });
    let status = child.wait().expect("wait rubash");
    (
        out_thread.join().unwrap_or_default(),
        err_thread.join().unwrap_or_default(),
        status.code(),
    )
}

/// Poll a file until its content equals `expected` or the deadline
/// passes; returns the last observed content.
fn wait_file_content(path: &Path, expected: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut last = String::new();
    while Instant::now() < deadline {
        if let Ok(content) = std::fs::read_to_string(path) {
            last = content;
            if last == expected {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    last
}

// ---------------- #2196 ----------------

#[test]
fn issue2196_numbered_fd_output_procsub_consumer_spawns() {
    // `cat 3> >(wc -c) <<< hi`: the numbered-fd entry is
    // list-only; its consumer must still run (it reads an empty
    // carrier here — cat writes the payload to stdout, not fd 3).
    let dir = test_dir("numbered-spawn");
    let count = dir.join("count");
    let script = format!("cat 3> >(wc -c > {}) <<< hi\nsleep 0.5\n", count.display());
    let (stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "hi\n");
    let content = wait_file_content(&count, "0\n");
    assert_eq!(content, "0\n");
}

#[test]
fn issue2196_numbered_fd_procsub_carries_payload() {
    // Writing to the numbered fd feeds the consumer: `>&3` dups
    // fd 1 onto the substitution pipe after `3> >(...)` binds it.
    let dir = test_dir("numbered-payload");
    let count = dir.join("count");
    let script = format!("echo hi 3> >(wc -c > {}) >&3\nsleep 0.5\n", count.display());
    let (_stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let content = wait_file_content(&count, "3\n");
    assert_eq!(content, "3\n");
}

#[test]
fn issue2196_higher_numbered_fd_procsub_carries_payload() {
    // Same mechanism on fd 7 (not just the fd-3 slot).
    let dir = test_dir("numbered-seven");
    let count = dir.join("count");
    let script = format!("echo hi 7> >(wc -c > {}) >&7\nsleep 0.5\n", count.display());
    let (_stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let content = wait_file_content(&count, "3\n");
    assert_eq!(content, "3\n");
}

#[test]
fn issue2196_numbered_fd_rebind_plain_then_procsub() {
    // Two entries bind fd 3 in sequence: the plain file is
    // created (and left empty), then the procsub entry rebinds
    // the fd and receives the payload.
    let dir = test_dir("numbered-rebind");
    let first = dir.join("first");
    let count = dir.join("count");
    let script = format!(
        "echo hi 3>{} 3> >(wc -c > {}) >&3\nsleep 0.5\n",
        first.display(),
        count.display()
    );
    let (_stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let content = wait_file_content(&count, "3\n");
    assert_eq!(content, "3\n");
    let first_content = std::fs::read_to_string(&first).unwrap_or_else(|_| "<missing>".into());
    assert_eq!(first_content, "");
}
