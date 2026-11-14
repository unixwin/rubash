//! rubash#3578 — a redirect-only `exec` carrying TWO (or more)
//! heredoc / here-string entries in one list bound only the LAST
//! one: `exec 14<<A 15<<B` left fd 14 unbound while fd 15 stood.
//!
//! GNU redir.c: do_redirections (redir.c:237) walks the whole list
//! and materializes EACH entry's own body (here_document_to_fd,
//! redir.c:423) before dup2'ing it onto its redirector (redir.c:1120),
//! so every entry binds. The old exec path resolved the heredoc
//! binding with a single `.rev().find()` over `heredoc_redirects`
//! and bound only that one entry.
//!
//! Expectations byte-verified against GNU bash 5.3.20 with
//! script-file probes under `ulimit -n 16`, mirroring the issue3195
//! pin's harness. The seed shape is the multi-below case from the
//! PR #3576 captain three-way probe (base==head, limit-independent).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue3578-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Run `body` as a script FILE under rubash with cwd = dir.
/// Returns (stdout, stderr).
fn rubash_script(dir: &Path, name: &str, body: &str) -> (String, String) {
    let script = dir.join(name);
    fs::write(&script, body).expect("write script fixture");
    let output = Command::new(RUBASH)
        .arg(&script)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn issue3578_exec_two_heredocs_bind_both() {
    // The issue's seed shape: both descriptors stay bound to their
    // own bodies.
    let dir = scratch("exec-two");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16\nexec 14<<A 15<<B\nfirst\nA\nsecond\nB\nread -u 14 x; read -u 15 y; echo \"x=$x y=$y\"\n",
    );
    assert_eq!(stdout, "x=first y=second\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue3578_exec_three_heredocs_bind_all() {
    let dir = scratch("exec-three");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16\nexec 13<<A 14<<B 15<<C\none\nA\ntwo\nB\nthree\nC\nread -u 13 a; read -u 14 b; read -u 15 c; echo \"a=$a b=$b c=$c\"\n",
    );
    assert_eq!(stdout, "a=one b=two c=three\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue3578_exec_same_fd_heredoc_last_wins() {
    // Two entries on one descriptor: GNU's ordered walk lets the
    // later entry overwrite the earlier one.
    let dir = scratch("exec-same-fd");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16\nexec 14<<A 14<<B\nfirst\nA\nsecond\nB\nread -u 14 x; echo \"x=$x\"\n",
    );
    assert_eq!(stdout, "x=second\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue3578_exec_heredoc_and_herestring_bind_both() {
    let dir = scratch("exec-mixed");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16\nexec 14<<A 15<<<\"strbody\"\nfirst\nA\nread -u 14 x; read -u 15 y; echo \"x=$x y=$y\"\n",
    );
    assert_eq!(stdout, "x=first y=strbody\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue3578_exec_stdin_and_numbered_heredoc_bind_both() {
    // An fd-0 heredoc and a numbered one in the same list: both
    // persist (the fd-0 body becomes the shell's stdin).
    let dir = scratch("exec-stdin");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16\nexec <<A 14<<B\nzero\nA\nfourteen\nB\nread z; read -u 14 x; echo \"z=$z x=$x\"\n",
    );
    assert_eq!(stdout, "z=zero x=fourteen\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue3578_exec_fd_var_and_numbered_heredoc_bind_both() {
    // A `{var}` heredoc and a numbered one in the same list: the
    // var allocation and the numbered bind do not crowd each other
    // out.
    let dir = scratch("exec-fd-var");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16\nexec {v}<<A 14<<B\nvarbody\nA\nfourteen\nB\nread -u \"$v\" x; read -u 14 y; echo \"x=$x y=$y\"\nif [ \"$v\" -lt 16 ]; then echo below; fi\n",
    );
    assert_eq!(stdout, "x=varbody y=fourteen\nbelow\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue3578_group_two_heredocs_bind_both_control() {
    // Control: the same two bodies on a brace group's redirect list
    // already bound both (bind_numbered_heredoc_redirects iterates
    // the whole list) — the loss was specific to redirect-only exec.
    let dir = scratch("group-two");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16\n{ read -u 14 x; read -u 15 y; echo \"x=$x y=$y\"; } 14<<A 15<<B\nfirst\nA\nsecond\nB\n",
    );
    assert_eq!(stdout, "x=first y=second\n");
    assert_eq!(stderr, "");
}
