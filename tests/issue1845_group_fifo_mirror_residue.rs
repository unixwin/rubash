//! rubash#1845 quantification pins: after a brace group reads from a
//! fifo, the session's FUNCTION_STDIN mirror must show NO residue —
//! later plain reads in the same session (from the script's stdin,
//! whether a regular file or a pipe) behave exactly as GNU bash 5.3.20.
//!
//! The 2026-10-27 quantification (see the w2-fd-trio cluster REPORT)
//! found the CommandFd0InputGuard snapshot/restore exact across these
//! cells; the pins guard that result. The fifo stream-level divergence
//! the same quantification found (bind slurps to EOF) is a separate
//! mechanism and is NOT pinned here.
//!
//! Unix-only: the shapes need mkfifo.

#![cfg(unix)]

use std::io::Write;
use std::process::{Command, Stdio};

fn run(script: &str, stdin_data: &str, pipe_stdin: bool) -> (String, String, Option<i32>) {
    let shell = env!("CARGO_BIN_EXE_rubash");
    let mut cmd = Command::new(shell);
    cmd.arg("-c")
        .arg(script)
        .env("LC_ALL", "C.UTF-8")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if pipe_stdin {
        cmd.stdin(Stdio::piped());
    } else {
        // Regular-file stdin: stage the bytes in a scratch file the
        // child inherits as fd 0.
        let path = std::env::temp_dir().join(format!(
            "rubash1845-stdin-{}-{}",
            std::process::id(),
            stdin_data.len()
        ));
        std::fs::write(&path, stdin_data).expect("write stdin scratch");
        let file = std::fs::File::open(&path).expect("open stdin scratch");
        cmd.stdin(Stdio::from(file));
        let child = cmd.spawn().expect("spawn rubash");
        let out = child.wait_with_output().expect("wait rubash");
        let _ = std::fs::remove_file(&path);
        return (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code(),
        );
    }
    let mut child = cmd.spawn().expect("spawn rubash");
    child
        .stdin
        .take()
        .expect("stdin pipe")
        .write_all(stdin_data.as_bytes())
        .expect("feed stdin");
    let out = child.wait_with_output().expect("wait rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

fn fifo_script(tag: &str, body: &str) -> String {
    format!(
        "F=/tmp/rubash1845-{tag}-$$.fifo; mkfifo \"$F\"; \
         ( printf 'fifo1\\nfifo2\\nfifo3\\n' > \"$F\" ) & \
         {body}; \
         rm -f \"$F\""
    )
}

#[test]
fn issue1845_group_fifo_partial_read_then_plain_reads_file_stdin() {
    let script = fifo_script(
        "a",
        "{ read a; read b; } < \"$F\"; echo \"a=$a b=$b\"; \
         read c; echo \"c=$c rc=$?\"; read d; echo \"d=$d rc=$?\"",
    );
    let (stdout, stderr, code) = run(&script, "stdin1\nstdin2\n", false);
    assert_eq!(stderr, "", "{stderr:?}");
    assert_eq!(stdout, "a=fifo1 b=fifo2\nc=stdin1 rc=0\nd=stdin2 rc=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1845_group_fifo_partial_read_then_plain_reads_pipe_stdin() {
    let script = fifo_script(
        "ap",
        "{ read a; read b; } < \"$F\"; echo \"a=$a b=$b\"; \
         read c; echo \"c=$c rc=$?\"; read d; echo \"d=$d rc=$?\"",
    );
    let (stdout, stderr, code) = run(&script, "stdin1\nstdin2\n", true);
    assert_eq!(stderr, "", "{stderr:?}");
    assert_eq!(stdout, "a=fifo1 b=fifo2\nc=stdin1 rc=0\nd=stdin2 rc=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1845_group_fifo_then_file_group_then_plain_read() {
    // A second group bound to a regular file (read to EOF, plus one
    // EOF read) must not disturb the session's later plain read either.
    let script = fifo_script(
        "b",
        "{ read a; } < \"$F\"; echo \"a=$a\"; \
         printf 'one\\ntwo\\nthree\\n' > /tmp/rubash1845-b-$$.txt; \
         { read x; read y; read z; read w; } < /tmp/rubash1845-b-$$.txt; \
         echo \"x=$x y=$y z=$z wrc=$?\"; \
         read v; echo \"v=$v rc=$?\"; rm -f /tmp/rubash1845-b-$$.txt",
    );
    let (stdout, stderr, code) = run(&script, "stdin1\n", false);
    assert_eq!(stderr, "", "{stderr:?}");
    assert_eq!(
        stdout,
        "a=fifo1\nx=one y=two z=three wrc=1\nv=stdin1 rc=0\n"
    );
    assert_eq!(code, Some(0));
}
