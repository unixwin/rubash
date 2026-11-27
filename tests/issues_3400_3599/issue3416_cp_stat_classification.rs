//! rubash#3416 — the emulated `cp`'s operand stat classification:
//! a source (or multi-source target) spelled THROUGH a regular
//! file (`a/b` with `a` a plain file) fails its path walk at the
//! non-directory component, so GNU coreutils reports the walk's
//! ENOTDIR — `cp: cannot stat 'a/b': Not a directory`. The base
//! code gated on the boolean `CpNode::exists()` and printed a
//! hardcoded ENOENT text for every failure, losing the class.
//!
//! The fix routes both operand gates through
//! `CpNode::stat_classified`, which keeps the stat's own error
//! and re-derives the walk verdict for a bare NotFound (the
//! Windows face answers ERROR_PATH_NOT_FOUND for both shapes):
//! the deepest existing proper ancestor decides — non-directory
//! means the walk died there (ENOTDIR); a directory means the
//! missing component is genuinely absent (ENOENT stands).
//!
//! Oracle: GNU coreutils 9.4 (probe-3416.sh on the lane tree;
//! base rubash measured on master f6e8c674):
//!   cp a/b dest        -> rc 1 ENOTDIR text (base: ENOENT text)
//!   cp -r a/b dest     -> rc 1 ENOTDIR text (base: ENOENT text)
//!   cp a/b/c dest      -> rc 1 ENOTDIR text (base: ENOENT text)
//!   cp f1 f2 a/b       -> rc 1 `target 'a/b': Not a directory`
//!                         (base: ENOENT text)
//!   cp missing dest    -> rc 1 ENOENT text (both; lock)
//!   cp -r missing dest -> rc 1 ENOENT text (both; lock)
//!   cp f1 f2 nodir     -> rc 1 `target 'nodir': No such file or
//!                         directory` (both; lock)
//!   cp a dest          -> rc 0, dest carries a's bytes (lock)
//! Family conservation (same probe round; these faces already
//! classified through their real stat/open errors at base and
//! must not move):
//!   rm a/b             -> `rm: cannot remove 'a/b': Not a directory`
//!   rm -f a/b          -> rc 0, silent (gnulib counts ENOTDIR
//!                         with ENOENT under -f)
//!   touch a/b          -> `touch: cannot touch 'a/b': Not a directory`
//!   cat a/b            -> `cat: a/b: Not a directory`
//!
//! These pins assert completion forms (stderr bytes + exit
//! status + physical landings), never timing. Unix only: the
//! emulated file builtins' anchor machinery is a Unix face in
//! the suite (the classification itself is platform-neutral).

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct ScratchDir(PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A scratch root holding `a` (regular file), `f1`/`f2` (regular
/// files) and `d/f` (a real directory with a member).
fn scratch(case: &str) -> ScratchDir {
    let dir = std::env::temp_dir().join(format!("rubash-3416-{case}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("d")).expect("create d");
    std::fs::write(dir.join("a"), "A\n").expect("write a");
    std::fs::write(dir.join("f1"), "F1\n").expect("write f1");
    std::fs::write(dir.join("f2"), "F2\n").expect("write f2");
    std::fs::write(dir.join("d/f"), "DF\n").expect("write d/f");
    let canonical = std::fs::canonicalize(&dir).expect("canonicalize scratch");
    ScratchDir(canonical)
}

/// Spawn rubash on `script` inside `dir`; returns (stdout, stderr,
/// status code).
fn run_rubash(dir: &Path, script: &str) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg("-c")
        .arg(script)
        .current_dir(dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        match child.try_wait().expect("poll rubash") {
            Some(status) => break status,
            None => {
                assert!(Instant::now() < deadline, "rubash timed out: {script}");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    };
    let output = child.wait_with_output().expect("collect rubash output");
    assert_eq!(output.status.code(), status.code());
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        status.code(),
    )
}

fn assert_shape(dir: &Path, script: &str, want_rc: i32, want_stderr: &str) {
    let (_stdout, stderr, rc) = run_rubash(dir, script);
    assert_eq!(rc, Some(want_rc), "rc for {script:?} (stderr {stderr:?})");
    assert_eq!(stderr, want_stderr, "stderr for {script:?}");
}

#[test]
fn cp_through_file_source_stat_is_enotdir() {
    let dir = scratch("src");
    assert_shape(
        &dir.0,
        "cp a/b dest",
        1,
        "cp: cannot stat 'a/b': Not a directory\n",
    );
    assert_shape(
        &dir.0,
        "cp -r a/b dest",
        1,
        "cp: cannot stat 'a/b': Not a directory\n",
    );
    assert_shape(
        &dir.0,
        "cp a/b/c dest",
        1,
        "cp: cannot stat 'a/b/c': Not a directory\n",
    );
    assert_shape(
        &dir.0,
        "cp -r a/b d",
        1,
        "cp: cannot stat 'a/b': Not a directory\n",
    );
}

#[test]
fn cp_through_file_target_is_enotdir() {
    let dir = scratch("tgt");
    assert_shape(
        &dir.0,
        "cp f1 f2 a/b",
        1,
        "cp: target 'a/b': Not a directory\n",
    );
}

#[test]
fn cp_missing_operands_stay_enoent() {
    let dir = scratch("missing");
    assert_shape(
        &dir.0,
        "cp missing dest",
        1,
        "cp: cannot stat 'missing': No such file or directory\n",
    );
    assert_shape(
        &dir.0,
        "cp -r missing dest",
        1,
        "cp: cannot stat 'missing': No such file or directory\n",
    );
    assert_shape(
        &dir.0,
        "cp f1 f2 nodir",
        1,
        "cp: target 'nodir': No such file or directory\n",
    );
    // A missing component under a real directory is likewise a
    // plain absence, never a walk death.
    assert_shape(
        &dir.0,
        "cp d/missing dest",
        1,
        "cp: cannot stat 'd/missing': No such file or directory\n",
    );
}

#[test]
fn cp_plain_copy_still_lands() {
    let dir = scratch("land");
    let (_stdout, stderr, rc) = run_rubash(&dir.0, "cp a dest");
    assert_eq!(rc, Some(0), "rc (stderr {stderr:?})");
    assert_eq!(stderr, "");
    assert_eq!(
        std::fs::read(dir.0.join("dest")).expect("dest landed"),
        b"A\n",
        "dest carries the source bytes"
    );
}

#[test]
fn family_stat_classification_conserved() {
    let dir = scratch("family");
    assert_shape(
        &dir.0,
        "rm a/b",
        1,
        "rm: cannot remove 'a/b': Not a directory\n",
    );
    assert_shape(&dir.0, "rm -f a/b", 0, "");
    assert_shape(
        &dir.0,
        "touch a/b",
        1,
        "touch: cannot touch 'a/b': Not a directory\n",
    );
    assert_shape(&dir.0, "cat a/b", 1, "cat: a/b: Not a directory\n");
}
