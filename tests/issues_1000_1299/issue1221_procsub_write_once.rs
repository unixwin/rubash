//! Issue unixwin/rubash#1221: a `>(while read...)` consumer's output was
//! printed twice — the body was fed the same bytes through TWO channels
//! (the fd-0 fd-table endpoint AND the FUNCTION_STDIN text mirror), so a
//! reader that exhausted fd 0 fell through and re-read the mirror.
//!
//! GNU owner: subst.c:6362 process_substitute() — the forked child's fd 0
//! is the single dup2'd pipe read end; there is no second transport. The
//! fix rides the detached-child spawn (issue #1220): the consumer's stdin
//! is one OS pipe and nothing else.
//!
//! Verified against WSL GNU Bash 5.3.20: `C:hello` prints once.

use std::io::Write;
use std::process::{Command, Stdio};

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
    let output = child.wait_with_output().expect("collect stdin run");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn while_read_consumer_prints_each_line_once() {
    // The trailing `sleep` gives the detached consumer time to land its
    // output before the script ends (GNU races the same way).
    let (stdout, stderr, code) = rubash_stdin(
        "echo hello > >(while read -r l; do echo \"C:$l\"; done)\nsleep 0.5\necho done\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout.matches("C:hello").count(), 1, "stdout: {stdout}");
    assert!(stdout.contains("done"), "stdout: {stdout}");
}

#[test]
fn multiline_while_read_consumer_no_duplicates() {
    let (stdout, stderr, code) = rubash_stdin(
        "printf \"a\\nb\\n\" > >(while read -r l; do echo \"C:$l\"; done)\nsleep 0.5\necho done\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout.matches("C:a").count(), 1, "stdout: {stdout}");
    assert_eq!(stdout.matches("C:b").count(), 1, "stdout: {stdout}");
}

#[test]
fn delayed_cat_consumer_prints_once() {
    // `>(sleep N; cat)` used to print the payload twice.
    let (stdout, stderr, code) = rubash_stdin("echo x > >(sleep 0.1; cat)\nsleep 0.5\necho done\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout.matches("x").count(), 1, "stdout: {stdout}");
}
