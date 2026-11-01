//! rubash#2409 — async-subshell stdin materialization leaked a NAMED
//! temp file per command.
//!
//! GNU's one materialized input, the heredoc temp of redir.c:423
//! here_document_to_fd, is "made really temporary" (redir.c:548-551):
//! written through a scratch handle, re-opened by name for reading,
//! and unlinked before the descriptor is ever used — it has no
//! directory entry for its whole working life (and small bodies ride
//! an anonymous pipe instead, redir.c:452-510). Rubash has two
//! compound_exec.rs materialization faces in the same class, and both
//! created the carrier by name and never unlinked it:
//!
//! 1. materialize_async_stdin_body — a background command carrying
//!    its own heredoc / here-string redirect (`cat <<EOF &`,
//!    `{ cat; } <<EOF &`, `( cat ) <<EOF &`).
//! 2. materialize_virtual_stdin_fd0 — an async child inheriting a
//!    virtual fd 0 (an enclosing compound's staged heredoc text
//!    channel / FUNCTION_STDIN, or a Text / ProcessSubstitution fd-0
//!    endpoint).
//!
//! Every such command left a `rubash-process-subst-*.tmp` in $TMPDIR
//! forever (the #2395 leak class, whose exec face was already fixed).
//! Both carriers are now anonymous from birth (unlinked after the
//! read re-open on Unix, FILE_FLAG_DELETE_ON_CLOSE on Windows).
//!
//! Expectations measured against GNU bash 5.3.20 with script files
//! (2026-11-01): GNU leaves the isolated TMPDIR empty and delivers
//! the stdin bytes unchanged.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rubash-issue2409-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let tmp = dir.join("tmp");
    fs::create_dir_all(&tmp).expect("create scratch tmp");
    (dir, tmp)
}

/// Run `script` as a script FILE under rubash, cwd = dir, TMPDIR =
/// the isolated tmp dir, stdin null. Returns (stdout, stderr).
fn rubash_script(dir: &Path, tmp: &Path, body: &str) -> (String, String) {
    let script = dir.join("case.sh");
    fs::write(&script, body).expect("write script fixture");
    let output = Command::new(RUBASH)
        .arg(&script)
        .current_dir(dir)
        .env("TMPDIR", tmp)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    assert!(
        output.status.success(),
        "script failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn tmp_listing(tmp: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(tmp)
        .expect("read scratch tmp")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// The residue rule: the isolated TMPDIR listing after the run is
/// exactly what it was before (empty) — no carrier of either face
/// survives under any name.
fn assert_no_residue(tmp: &Path) {
    assert_eq!(
        tmp_listing(tmp),
        Vec::<String>::new(),
        "temp residue left in TMPDIR"
    );
}

/// Face 1, simple-command form: `cat <<EOF &` delivers the body and
/// leaves nothing behind.
#[test]
fn issue2409_simple_async_heredoc_no_residue() {
    let (dir, tmp) = scratch("simple");
    let (stdout, _) = rubash_script(&dir, &tmp, "cat <<E1 &\nhello-A\nE1\nwait\n");
    assert_eq!(stdout, "hello-A\n");
    assert_no_residue(&tmp);
}

/// Face 1, brace-group forms from the issue seed: heredoc-fed groups
/// backgrounded; both bodies arrive (one through `read`) and no
/// carrier survives.
#[test]
fn issue2409_brace_group_async_heredoc_no_residue() {
    let (dir, tmp) = scratch("brace");
    let (stdout, _) = rubash_script(
        &dir,
        &tmp,
        "{ cat; } <<E2 &\nhello-B\nE2\nwait\n\
         { read line; echo \"got=$line\"; } <<E3 &\nworld-B\nE3\nwait\n",
    );
    assert_eq!(stdout, "hello-B\ngot=world-B\n");
    assert_no_residue(&tmp);
}

/// Face 1, subshell form: `( cat ) <<EOF &`.
#[test]
fn issue2409_subshell_async_heredoc_no_residue() {
    let (dir, tmp) = scratch("subshell");
    let (stdout, _) = rubash_script(&dir, &tmp, "( cat ) <<E4 &\nhello-C\nE4\nwait\n");
    assert_eq!(stdout, "hello-C\n");
    assert_no_residue(&tmp);
}

/// Face 1, here-string variant: `cat <<<word &` rides the same
/// materializer (HereString arm).
#[test]
fn issue2409_async_here_string_no_residue() {
    let (dir, tmp) = scratch("herestring");
    let (stdout, _) = rubash_script(&dir, &tmp, "cat <<<string-body &\nwait\n");
    assert_eq!(stdout, "string-body\n");
    assert_no_residue(&tmp);
}

/// Face 2: the heredoc belongs to an enclosing in-process compound;
/// the async child inherits the staged virtual fd 0. The body still
/// arrives and the fd-0 carrier leaves no residue.
#[test]
fn issue2409_inherited_virtual_stdin_no_residue() {
    let (dir, tmp) = scratch("inherited");
    let (stdout, _) = rubash_script(&dir, &tmp, "{ ( cat ) & wait; } <<E5\nbody-D\nE5\n");
    assert_eq!(stdout, "body-D\n");
    assert_no_residue(&tmp);
}
