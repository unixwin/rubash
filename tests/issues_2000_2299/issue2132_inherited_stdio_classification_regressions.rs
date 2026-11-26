//! rubash#2132 — the fd-fact sync classified EVERY non-terminal
//! inherited stdin as a pipe: with the process's stdin = /dev/null,
//! `[ -p /dev/stdin ]` answered true and `[ -c /dev/stdin ]` false.
//! The ambient stdout/stderr sinks carried the same console-or-pipe
//! default (a regular-file stdout answered -p true, -f false).
//!
//! GNU model: test.c:544 unary_test → lib/sh/eaccess.c:116
//! sh_stataccess stats the descriptor the process actually holds.
//! The parent hands the shell a console, a pipe, a regular file,
//! /dev/null, a directory, or a socket, and the predicates answer
//! from THAT inode. The fix classifies the process's own stdio
//! handles by the handles themselves (console / pipe / socket /
//! disk-file referent / char-device class / directory referent) at
//! the three ambient arms of sync_fd_alias_facts.
//!
//! Expectations hand-measured against GNU bash 5.3.20 via script
//! files. The heredoc/here-string pins at the bottom are GUARDS for
//! the #2131×#2132 interaction: the old always-pipe default answered
//! them correctly by luck, and the truthful ambient classification
//! must not regress the text feeds (their pipe fact is the staged
//! mirror's own statement, #2131's FD0_MIRROR_FACT).

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn fixture_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2132-pins-{tag}"));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    dir
}

fn collect(output: std::process::Output) -> (String, String, Option<i32>) {
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn run_c(script: &str, stdin: Stdio, tag: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .stdin(stdin)
        .current_dir(fixture_dir(tag))
        .output()
        .expect("run rubash");
    collect(output)
}

fn run_c_pipe_stdin(script: &str, tag: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir(fixture_dir(tag))
        .spawn()
        .expect("spawn rubash");
    // One line then EOF: the child sees a genuine pipe as its stdin.
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(b"x\n")
        .expect("write pipe");
    collect(child.wait_with_output().expect("wait rubash"))
}

fn verdicts(stdout: &str) -> Vec<(String, bool)> {
    stdout
        .lines()
        .map(|line| {
            let (name, verdict) = line.rsplit_once('=').expect("verdict line NAME=T/F");
            (name.to_string(), verdict == "T")
        })
        .collect()
}

fn assert_verdicts(stdout: &str, expected: &[(&str, bool)]) {
    let got = verdicts(stdout);
    assert_eq!(got.len(), expected.len(), "verdict count, got: {stdout}");
    for ((name, value), (want_name, want)) in got.iter().zip(expected) {
        assert_eq!(name, want_name, "verdict order, got: {stdout}");
        assert_eq!(value, want, "verdict {name}, got: {stdout}");
    }
}

const AMBIENT_PROBES: &str = "[ -p /dev/stdin ] && echo p=T || echo p=F\n\
     [ -c /dev/stdin ] && echo c=T || echo c=F\n\
     [ -f /dev/stdin ] && echo f=T || echo f=F\n\
     [ -s /dev/stdin ] && echo s=T || echo s=F\n\
     [ -w /dev/stdin ] && echo w=T || echo w=F\n\
     [ -e /dev/stdin ] && echo e=T || echo e=F\n";

#[test]
fn issue2132_inherited_null_stdin_is_a_char_device_not_a_pipe() {
    // The filed reproducer ambient: process stdin is /dev/null.
    let (stdout, _stderr, code) = run_c(AMBIENT_PROBES, Stdio::null(), "null");
    assert_eq!(code, Some(0));
    assert_verdicts(
        &stdout,
        &[
            ("p", false),
            ("c", true),
            ("f", false),
            ("s", false),
            ("w", true),
            ("e", true),
        ],
    );
}

#[test]
fn issue2132_inherited_file_stdin_answers_from_the_file() {
    let dir = fixture_dir("file");
    let path = dir.join("ambient.txt");
    std::fs::write(&path, b"hello world\n").expect("write ambient file");
    let file = std::fs::File::open(&path).expect("open ambient file");
    let (stdout, _stderr, code) = run_c(AMBIENT_PROBES, Stdio::from(file), "file");
    assert_eq!(code, Some(0));
    assert_verdicts(
        &stdout,
        &[
            ("p", false),
            ("c", false),
            ("f", true),
            ("s", true),
            ("w", true),
            ("e", true),
        ],
    );
}

#[test]
fn issue2132_inherited_pipe_stdin_is_a_pipe() {
    let (stdout, _stderr, code) = run_c_pipe_stdin(AMBIENT_PROBES, "pipe");
    assert_eq!(code, Some(0));
    assert_verdicts(
        &stdout,
        &[
            ("p", true),
            ("c", false),
            ("f", false),
            ("s", false),
            ("w", true),
            ("e", true),
        ],
    );
}

#[test]
fn issue2132_inherited_directory_stdin_answers_from_the_directory() {
    let dir = fixture_dir("dir");
    let file = std::fs::File::open(&dir).expect("open ambient directory");
    let (stdout, _stderr, code) = run_c(
        "[ -d /dev/stdin ] && echo d=T || echo d=F\n\
         [ -f /dev/stdin ] && echo f=T || echo f=F\n\
         [ -p /dev/stdin ] && echo p=T || echo p=F\n",
        Stdio::from(file),
        "dir",
    );
    assert_eq!(code, Some(0));
    assert_verdicts(&stdout, &[("d", true), ("f", false), ("p", false)]);
}

#[test]
fn issue2132_compound_rebind_to_dev_null_under_every_ambient() {
    // The filed reproducer: the compound's own `< /dev/null` rebind —
    // not the ambient — decides, whatever the parent handed over.
    const SCRIPT: &str = "{ [ -p /dev/stdin ]; } < /dev/null && echo p=T || echo p=F\n\
         { [ -c /dev/stdin ]; } < /dev/null && echo c=T || echo c=F\n";
    let (stdout, _e, code) = run_c(SCRIPT, Stdio::null(), "rebind-null");
    assert_eq!(code, Some(0));
    assert_verdicts(&stdout, &[("p", false), ("c", true)]);
    let (stdout, _e, code) = run_c_pipe_stdin(SCRIPT, "rebind-pipe");
    assert_eq!(code, Some(0));
    assert_verdicts(&stdout, &[("p", false), ("c", true)]);
}

#[test]
fn issue2132_ambient_stdout_and_stderr_files_are_files_not_pipes() {
    let dir = fixture_dir("stdio");
    // stdout is a regular file: the verdicts ride stderr instead.
    let out_file = std::fs::File::create(dir.join("stdout.txt")).expect("create stdout file");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args([
            "-c",
            "[ -f /dev/stdout ] && echo out_f=T >&2 || echo out_f=F >&2\n\
                      [ -p /dev/stdout ] && echo out_p=T >&2 || echo out_p=F >&2",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::from(out_file))
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_verdicts(&stderr, &[("out_f", true), ("out_p", false)]);
    // stderr is a regular file: a dup of it onto fd 1 is a file too.
    let err_file = std::fs::File::create(dir.join("stderr.txt")).expect("create stderr file");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args([
            "-c",
            "[ -f /dev/stderr ] && echo err_f=T || echo err_f=F\n\
                      [ -p /dev/stderr ] && echo err_p=T || echo err_p=F\n\
                      [ -f /dev/stdout ] >&2 && echo dup_f=T || echo dup_f=F",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::from(err_file))
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let (stdout, _stderr, code) = collect(output);
    assert_eq!(code, Some(0));
    assert_verdicts(
        &stdout,
        &[("err_f", true), ("err_p", false), ("dup_f", true)],
    );
}

#[test]
fn issue2132_text_feeds_keep_their_pipe_fact_under_a_file_ambient() {
    // GUARD (#2131×#2132): heredoc / here-string feeds are pipes in
    // GNU whatever the ambient stdin is; their fact comes from the
    // staged mirror, not from the (now truthful) ambient entry.
    let dir = fixture_dir("feeds");
    let path = dir.join("ambient.txt");
    std::fs::write(&path, b"hello world\n").expect("write ambient file");
    let file = std::fs::File::open(&path).expect("open ambient file");
    let (stdout, _stderr, code) = run_c(
        "[ -p /dev/stdin ] <<< x && echo hs_p=T || echo hs_p=F\n\
         [ -f /dev/stdin ] <<< x && echo hs_f=T || echo hs_f=F\n\
         { [ -p /dev/stdin ]; } <<EOF && echo hd_p=T || echo hd_p=F\nbody\nEOF\n",
        Stdio::from(file),
        "feeds",
    );
    assert_eq!(code, Some(0));
    assert_verdicts(&stdout, &[("hs_p", true), ("hs_f", false), ("hd_p", true)]);
}
