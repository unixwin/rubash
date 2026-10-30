//! Issue unixwin/rubash#2200: coproc construct-level numbered-fd redirects
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

// ---------------- #2200 ----------------

#[test]
fn issue2200_coproc_construct_redirect_binds_numbered_fd() {
    // The issue's repro: `coproc CP { ...; } 3>fd3out.txt` — the
    // file is created by the redirect; under GNU the body's
    // `>&3` write lands in it. The stdout line still arrives on
    // the coproc pipe.
    let dir = test_dir("coproc-fd3");
    let fd3out = dir.join("fd3out.txt");
    let script = format!(
        "coproc CP {{ echo hello-fd3 >&3; echo hello-stdout; }} 3>{}\nread -r line <&${{CP[0]}}\necho \"PIPE:$line\"\nwait ${{CP_PID}}\n",
        fd3out.display()
    );
    let (stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "PIPE:hello-stdout\n", "stderr: {stderr:?}");
    assert!(
        !stderr.contains("Bad file descriptor"),
        "stderr: {stderr:?}"
    );
    let content = wait_file_content(&fd3out, "hello-fd3\n");
    assert_eq!(content, "hello-fd3\n");
}

#[test]
fn issue2200_coproc_construct_redirect_numbered_fd_input() {
    // The input sibling: `3<file` on the coproc construct binds
    // fd 3 in the body, whose `cat <&3` forwards the file's bytes
    // to the coproc pipe.
    let dir = test_dir("coproc-fd3in");
    let fd3in = dir.join("fd3in.txt");
    std::fs::write(&fd3in, "payload-in\n").expect("seed input file");
    let script = format!(
        "coproc CP {{ cat <&3; }} 3<{}\nread -r line <&${{CP[0]}}\necho \"GOT:$line\"\nwait ${{CP_PID}}\n",
        fd3in.display()
    );
    let (stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "GOT:payload-in\n", "stderr: {stderr:?}");
    assert!(
        !stderr.contains("Bad file descriptor"),
        "stderr: {stderr:?}"
    );
}

#[test]
fn issue2200_coproc_dup_from_pipe_binds_numbered_fd() {
    // `3>&1` on the construct: fd 1 in the body is the coproc
    // pipe, so the dup gives the body a second handle on its own
    // output channel — both lines arrive on the pipe, in order.
    let script = "coproc CP { echo via3 >&3; echo main; } 3>&1\nread -r a <&${CP[0]}\nread -r b <&${CP[0]}\necho \"A:$a B:$b\"\nwait ${CP_PID}\n";
    let (stdout, stderr, code) = rubash_stdin(script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "A:via3 B:main\n", "stderr: {stderr:?}");
    assert!(
        !stderr.contains("Bad file descriptor"),
        "stderr: {stderr:?}"
    );
}

#[test]
fn issue2200_async_construct_redirect_numbered_fd_control() {
    // Control (already working on master): the async respawn
    // transports construct-level fd>=3 bindings, so the same
    // shape under `&` lands the write. Pins the sibling site so
    // the coproc fix cannot regress it.
    let dir = test_dir("async-fd3");
    let async3 = dir.join("async3.txt");
    let script = format!("{{ echo async-fd3 >&3; }} 3>{} &\nwait\n", async3.display());
    let (_stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let content = wait_file_content(&async3, "async-fd3\n");
    assert_eq!(content, "async-fd3\n");
}
