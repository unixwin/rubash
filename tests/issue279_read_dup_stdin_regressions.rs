//! niubash#279: a descriptor dup'd from stdin (`exec 8<&0`) is the same
//! open file description as fd 0 (POSIX dup; redir.c do_redirections) and
//! must read like fd 0.
//!
//! On a real Windows console, `read <&8` after `exec 8<&0` used to return
//! rc=1 immediately and drop the keystroke: the `<&N` arm returned None for
//! the inherited-stdin endpoint instead of serving it from the transports
//! an fd 0 read uses. Non-tty dup reads are the deterministic leg (the
//! console leg is covered by scripts/smoke-read-tty-pty.py under ConPTY).

use std::io::Write;
use std::process::{Command, Stdio};

fn run_stdin(args: &[&str], stdin: &[u8]) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(stdin)
        .expect("write stdin");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// `exec 8<&0; read <&8` reads the next stdin record, exactly like `read`
/// on fd 0 (GNU dup semantics; probe: WSL 5.2.21 rc=0, k=[Q]).
#[test]
fn dup_fd_read_redirect_serves_stdin() {
    let (stdout, stderr, rc) = run_stdin(
        &["-c", "exec 8<&0; IFS= read -r k <&8; echo \"rc=$? k=[$k]\""],
        b"Q\n",
    );
    assert_eq!(stdout, "rc=0 k=[Q]\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
}

/// `read -u 8` after the dup is the same descriptor read (read.def:294
/// `-u fd` shape).
#[test]
fn dup_fd_read_u_serves_stdin() {
    let (stdout, stderr, rc) = run_stdin(
        &[
            "-c",
            "exec 8<&0; IFS= read -r -u 8 k; echo \"rc=$? k=[$k]\"",
        ],
        b"W\n",
    );
    assert_eq!(stdout, "rc=0 k=[W]\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
}

/// dup shares the file offset: the dup read and a plain fd 0 read see
/// consecutive records — the first key is not lost to either reader.
#[test]
fn dup_and_fd0_reads_share_offset() {
    let (stdout, stderr, rc) = run_stdin(
        &[
            "-c",
            "exec 8<&0; IFS= read -r a <&8; IFS= read -r b; echo \"a=[$a] b=[$b]\"",
        ],
        b"first\nsecond\n",
    );
    assert_eq!(stdout, "a=[first] b=[second]\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
}

/// A dup stays live across a `<file` loop on fd 0 (the #276 workaround
/// shape): rebinding fd 0 inside the loop must not kill the dup.
#[test]
fn dup_survives_fd0_file_loop() {
    let script = "\
printf 'a\\nb\\n' > f279.txt
exec 8<&0
while read q; do :; done < f279.txt
IFS= read -r k <&8; echo \"rc=$? k=[$k]\"
";
    let path = std::env::temp_dir().join(format!("rubash-279-{}.sh", std::process::id()));
    std::fs::write(&path, script).expect("write script");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(b"Z\n")
        .expect("write stdin");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait rubash");
    let _ = std::fs::remove_file(&path);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(stdout, "rc=0 k=[Z]\n", "stderr: {stderr:?}");
    assert_eq!(output.status.code(), Some(0), "stderr: {stderr:?}");
}
