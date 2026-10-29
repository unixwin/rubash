//! rubash#2111 — an empty pipeline payload is a bound fd 0 at EOF,
//! never an invitation to read the shell's inherited stdin.
//!
//! GNU execute_cmd.c execute_pipeline + do_piping (execute_cmd.c:6373)
//! replaces every element's fd 0 with the inter-stage pipe; an element
//! whose producer wrote nothing reads EOF from that pipe. Rubash stages
//! the payload as the FUNCTION_STDIN mirror, but the central remaining-
//! input judge (function_stdin_remaining) treated a present-but-empty
//! mirror as UNBOUND, so downstream consumers fell through to the
//! inherited process handle: `true | command cat` with the shell's
//! stdin on a file printed the file, and on a live pipe/terminal it
//! blocked. The same fall-through waited in the comsub, function-call
//! and exec-image stdin ladders that share the judge.
//!
//! The ruling (the FUNCTION_STDIN twin of rubash#2103's fd-table
//! ruling): PRESENCE of the mirror is the binding. Every staging site
//! inserts the pair only to bind fd 0, and the live-handle faces
//! (streaming tail, live stage 0) remove it instead. One consumer had
//! already patched around the emptiness judge with exactly this
//! presence check (read_inherited_process_stdin_to_string, rubash#436).
//!
//! Oracle: GNU bash 5.3.20, script-file form. Every expectation below
//! is byte-verified against it (matrix in the fix workspace).

use std::process::{Command, Stdio};

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2111-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn payload_file(tag: &str) -> std::path::PathBuf {
    let path = temp_dir(tag).join("payload.txt");
    std::fs::write(&path, "file-content\n").expect("write payload");
    path
}

/// Run `script` with the child's stdin attached to `stdin_file`.
fn rubash_with_stdin_file(
    script: &str,
    stdin_file: &std::path::Path,
) -> (String, String, Option<i32>) {
    let file = std::fs::File::open(stdin_file).expect("open stdin file");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .stdin(Stdio::from(file))
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[track_caller]
fn assert_gnu_with_file_stdin(script: &str, tag: &str, expected_stdout: &str) {
    let payload = payload_file(tag);
    let (stdout, stderr, code) = rubash_with_stdin_file(script, &payload);
    assert_eq!(
        stdout, expected_stdout,
        "stdout diverged from GNU 5.3.20 for: {script}"
    );
    assert_eq!(stderr, "", "unexpected stderr for: {script}");
    assert_eq!(code, Some(0), "exit status diverged for: {script}");
}

#[test]
fn empty_payload_command_cat_does_not_steal_file_stdin() {
    // The reported face: the producer wrote nothing, so the stage's
    // fd 0 is an empty pipe — EOF — not the shell's file stdin.
    assert_gnu_with_file_stdin("true | command cat", "cmdcat", "");
}

#[test]
fn empty_payload_exec_cat_does_not_steal_file_stdin() {
    // The exec-image ladder shares the same judge: the image that
    // replaces the stage shell inherits the pipe, not the file.
    assert_gnu_with_file_stdin("true | exec cat", "execcat", "");
}

#[test]
fn empty_payload_second_command_stage_does_not_steal() {
    assert_gnu_with_file_stdin("true | command cat | command cat", "second", "");
}

#[test]
fn empty_payload_function_stage_does_not_steal() {
    assert_gnu_with_file_stdin("f() { command cat; }; true | f", "funcstage", "");
}

#[test]
fn empty_payload_comsub_stage_does_not_steal() {
    assert_gnu_with_file_stdin(
        "v=$(true | command cat); echo \"v=[$v]\"",
        "comsub",
        "v=[]\n",
    );
}

#[test]
fn empty_heredoc_group_command_cat_is_bound_empty() {
    // A present-but-empty binding from a heredoc (not a pipeline):
    // same presence ruling outside the stage machinery.
    assert_gnu_with_file_stdin("{ command cat; } <<EOF\nEOF", "emptyheredoc", "");
}

#[test]
fn nonempty_payload_still_flows_to_command_cat() {
    assert_gnu_with_file_stdin("echo hi | command cat", "nonempty", "hi\n");
}

#[test]
fn nonempty_heredoc_group_command_cat_reads_body() {
    assert_gnu_with_file_stdin(
        "{ command cat; } <<EOF\nbody-line\nEOF",
        "bodyheredoc",
        "body-line\n",
    );
}

#[test]
fn toplevel_command_cat_without_mirror_still_reads_stdin() {
    // No staged mirror anywhere: fd 0 genuinely is the inherited
    // handle, and the file MUST be read (overcorrection guard).
    assert_gnu_with_file_stdin("command cat", "toplevel", "file-content\n");
}

#[test]
fn sibling_2103_exec_devnull_stage_still_reads_pipe() {
    // rubash#2103 control: the fd-table presence ruling must not
    // regress — a persistent /dev/null binding stays invisible to a
    // later stage, whose `read` draws the pipe.
    assert_gnu_with_file_stdin(
        "exec 0</dev/null; printf y | { read l; echo \"stage-read=[$l]\"; }",
        "c2103a",
        "stage-read=[y]\n",
    );
}

#[test]
fn sibling_2103_exec_file_offset_still_preserved() {
    // rubash#2103 control: the stage reads the pipe, the persistent
    // file binding keeps its offset for the next shell read.
    let dir = temp_dir("c2103b");
    let script = format!(
        "printf 'l1\\nl2\\n' > \"{0}/off.txt\"; exec 0<\"{0}/off.txt\"; read first; \
         printf y | {{ read l; echo \"stage=[$l]\"; }}; read third; \
         echo \"first=[$first] third=[$third]\"",
        dir.display()
    );
    assert_gnu_with_file_stdin(&script, "c2103b", "stage=[y]\nfirst=[l1] third=[l2]\n");
}

/// The blocking face: with the shell's stdin a pipe whose writer stays
/// open but silent, an empty-payload stage must return at once (EOF),
/// not block on the inherited handle. Bounded by a channel timeout so
/// a regression fails instead of hanging the suite.
#[cfg(unix)]
#[test]
fn empty_payload_stage_does_not_block_on_open_pipe_stdin() {
    let dir = temp_dir("fifo");
    let fifo = dir.join("stdin.fifo");
    let mkfifo = Command::new("mkfifo").arg(&fifo).status().expect("mkfifo");
    assert!(mkfifo.success(), "mkfifo failed");
    // Writer: opens the fifo (unblocking the reader open below) and
    // then holds it open, silent, far past the test's bound.
    let mut writer = Command::new("sh")
        .arg("-c")
        .arg("exec 9>\"$1\"; sleep 30")
        .arg("sh")
        .arg(&fifo)
        .spawn()
        .expect("spawn fifo writer");
    let read_end = std::fs::File::open(&fifo).expect("open fifo read end");
    let (tx, rx) = std::sync::mpsc::channel();
    let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("true | command cat")
        .stdin(Stdio::from(read_end))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    let finished = rx.recv_timeout(std::time::Duration::from_secs(10));
    let _ = writer.kill();
    let _ = writer.wait();
    let output = finished
        .expect("rubash blocked on the inherited pipe stdin (10s bound)")
        .expect("wait_with_output");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}
