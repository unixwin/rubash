//! issue#527 regression: `-r` must enter the restricted shell through every
//! option order, not only when it precedes `-s`. GNU bash scans argv left to
//! right (shell.c parse_short_options marks `read_from_stdin` for `-s` and
//! keeps parsing the remaining argv), so both `bash -r -s` and `bash -s -r`
//! land restricted ($- carries r, cd is refused), and any non-option words
//! after `-s` become positional parameters ($0 stays the shell name).
//! rubash executed the stdin script the moment it saw `-s`, so the trailing
//! `-r` was silently swallowed as a positional and the shell ran
//! unrestricted.
//!
//! Oracle: GNU bash 5.3.0 (WSL) 2026-10-12 — `bash -s -r` with
//! `echo flags=$-; cd /` prints `flags=hrBs` and refuses cd (rc 1);
//! `bash -s -r a b` sets $1=a $2=b; `bash -s a b` stays unrestricted with
//! the same positionals; `bash -s /no/such/file` still reads stdin and
//! makes the word $1.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn shell_bin() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_rubash") {
        return PathBuf::from(path);
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(profile)
        .join(if cfg!(windows) {
            "rubash.exe"
        } else {
            "rubash"
        })
}

struct Output {
    status: i32,
    stdout: String,
    stderr: String,
}

/// Run rubash with `args` and `script` on stdin; capture the three channels.
fn run_stdin(args: &[&str], script: &str) -> Output {
    let mut child = Command::new(shell_bin())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    {
        let mut stdin = child.stdin.take().expect("stdin");
        stdin.write_all(script.as_bytes()).expect("write script");
    }
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait for rubash");
    Output {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).replace('\r', ""),
    }
}

/// The issue's second entry path: `-r` after `-s` must still restrict.
#[test]
fn r_after_s_enters_restricted_mode() {
    let out = run_stdin(&["-s", "-r"], "echo flags=$-\ncd / && echo cd-ok=$?\n");
    assert_eq!(out.stdout, "flags=hrBs\n", "-r after -s must restrict");
    assert!(
        out.stderr.contains("restricted"),
        "cd must be refused, stderr: {:?}",
        out.stderr
    );
    assert_eq!(out.status, 1, "refused cd leaves rc 1");
}

/// The first entry path stays intact: `-r` before `-s` still restricts.
#[test]
fn r_before_s_still_restricted() {
    let out = run_stdin(&["-r", "-s"], "echo flags=$-\ncd /\n");
    assert_eq!(out.stdout, "flags=hrBs\n");
    assert!(out.stderr.contains("restricted"));
}

/// Words after `-s -r` become positional parameters while the shell stays
/// restricted ($1/$2 skip the consumed options; GNU `bash -s -r a b`).
#[test]
fn s_r_then_words_become_positionals() {
    let out = run_stdin(&["-s", "-r", "a", "b"], "echo flags=$-\necho p1=$1 p2=$2\n");
    assert_eq!(out.stdout, "flags=hrBs\np1=a p2=b\n");
}

/// Plain `-s a b` keeps its positional behavior and stays unrestricted.
#[test]
fn plain_s_words_stay_unrestricted_positionals() {
    let out = run_stdin(&["-s", "a", "b"], "echo flags=$-\ncd /\necho p1=$1 p2=$2\n");
    assert!(
        !out.stdout.contains("flags=hrBs\n"),
        "no r without -r: {:?}",
        out.stdout
    );
    assert!(!out.stderr.contains("restricted"), "cd must be allowed");
    assert!(out.stdout.contains("p1=a p2=b"), "{:?}", out.stdout);
}

/// `-s` with a word that looks like a script path still reads stdin; the
/// word becomes a positional (GNU never falls back to the file after -s).
#[test]
fn s_with_script_looking_word_still_reads_stdin() {
    let phantom = "Z:/rubash-issue527-no-such-dir/no-such-script";
    let out = run_stdin(&["-s", phantom], "echo stdin-ran\necho p1=$1\n");
    assert!(
        out.stdout.contains("stdin-ran"),
        "stdin must be read, got {:?}",
        out.stdout
    );
    assert!(out.stdout.contains(&format!("p1={phantom}")));
}
