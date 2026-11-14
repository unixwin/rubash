//! rubash#3425 — the emulated `chmod`'s operand stat classification:
//! a mode operand spelled THROUGH a regular file (`a/b` with `a` a
//! plain file) fails its path walk at the non-directory component,
//! so GNU coreutils reports the walk's ENOTDIR — `chmod: cannot
//! access 'a/b': Not a directory`. The base code discarded the
//! `std::fs::metadata` error in its apply arms and printed a
//! hardcoded ENOENT text for every failure, losing the class — even
//! on unix, where the kernel error already carries ENOTDIR.
//!
//! The fix routes the operand stat through the `CpNode` machinery
//! (the same root external_cp builds: `cp_operand_root` gives an
//! anchor-relative root under a dead cwd, else the plain joined
//! path), and renders the apply-arm failure with
//! `CpNode::stat_classified` + `posix_errors::message`. The
//! re-derivation keeps the stat's own error and re-derives the walk
//! verdict for a bare NotFound (the Windows face answers
//! ERROR_PATH_NOT_FOUND for both shapes): the deepest existing
//! proper ancestor decides — non-directory means the walk died
//! there (ENOTDIR); a directory means the missing component is
//! genuinely absent (ENOENT stands).
//!
//! Oracle: GNU coreutils 9.4 (probe-3425.sh on the lane tree;
//! `chmod.c:231` — `error (0, ent->fts_errno, _("cannot access
//! %s"))`, i.e. fts's walk errno verbatim):
//!   chmod 600 a/b      -> rc 1 ENOTDIR text (base: ENOENT text)
//!   chmod 600 a/b/c    -> rc 1 ENOTDIR text (base: ENOENT text)
//!   chmod 600 a/b d    -> rc 1, only the a/b line (base: ENOENT)
//!   chmod 600 f1 f2 a/b-> rc 1 ENOTDIR text (base: ENOENT text)
//!   chmod +x a/b       -> rc 1 ENOTDIR text (base: ENOENT text)
//!   chmod 600 a/       -> rc 1 ENOTDIR text (base: ENOENT text)
//!   chmod 600 missing  -> rc 1 ENOENT text (both; lock)
//!   chmod 600 missing/sub -> rc 1 ENOENT text (both; lock)
//!   chmod 600 d/missing-> rc 1 ENOENT text (both; lock)
//!   chmod 600 a        -> rc 0, mode 600 (both; lock)
//!   chmod 600 d/f      -> rc 0, mode 600 (both; lock)
//!   chmod 600 d        -> rc 0, mode 600 (both; lock)
//!
//! These pins assert completion forms (stderr bytes + exit status
//! + physical modes), never timing. Unix only: the emulated file
//! builtins' anchor machinery is a Unix face in the suite (the
//! classification itself is platform-neutral).

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
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
    let dir = std::env::temp_dir().join(format!("rubash-3425-{case}-{}", std::process::id()));
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

fn assert_mode(dir: &Path, rel: &str, want: u32) {
    let mode = std::fs::metadata(dir.join(rel))
        .expect("stat landed operand")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, want, "mode of {rel}");
}

#[test]
fn chmod_through_file_reports_enotdir() {
    let dir = scratch("through");
    assert_shape(
        &dir.0,
        "chmod 600 a/b",
        1,
        "chmod: cannot access 'a/b': Not a directory\n",
    );
    assert_shape(
        &dir.0,
        "chmod 600 a/b/c",
        1,
        "chmod: cannot access 'a/b/c': Not a directory\n",
    );
}

#[test]
fn chmod_through_file_multi_operand_reports_enotdir() {
    let dir = scratch("multi");
    // Only the through-file operand fails; the directory operand
    // still lands, and only the failing line is printed.
    assert_shape(
        &dir.0,
        "chmod 600 a/b d",
        1,
        "chmod: cannot access 'a/b': Not a directory\n",
    );
    assert_mode(&dir.0, "d", 0o600);
    // Restore the directory so the scratch teardown can remove it.
    let (_stdout, stderr, rc) = run_rubash(&dir.0, "chmod 755 d");
    assert_eq!(rc, Some(0), "rc (stderr {stderr:?})");
    assert_eq!(stderr, "");
    assert_shape(
        &dir.0,
        "chmod 600 f1 f2 a/b",
        1,
        "chmod: cannot access 'a/b': Not a directory\n",
    );
}

#[test]
fn chmod_missing_operands_stay_enoent() {
    let dir = scratch("missing");
    assert_shape(
        &dir.0,
        "chmod 600 missing",
        1,
        "chmod: cannot access 'missing': No such file or directory\n",
    );
    assert_shape(
        &dir.0,
        "chmod 600 missing/sub",
        1,
        "chmod: cannot access 'missing/sub': No such file or directory\n",
    );
    // A missing component under a real directory is likewise a
    // plain absence, never a walk death.
    assert_shape(
        &dir.0,
        "chmod 600 d/missing",
        1,
        "chmod: cannot access 'd/missing': No such file or directory\n",
    );
}

#[test]
fn chmod_plain_operands_still_land() {
    let dir = scratch("land");
    let (_stdout, stderr, rc) = run_rubash(&dir.0, "chmod 600 a");
    assert_eq!(rc, Some(0), "rc (stderr {stderr:?})");
    assert_eq!(stderr, "");
    assert_mode(&dir.0, "a", 0o600);
    let (_stdout, stderr, rc) = run_rubash(&dir.0, "chmod 600 d/f");
    assert_eq!(rc, Some(0), "rc (stderr {stderr:?})");
    assert_eq!(stderr, "");
    assert_mode(&dir.0, "d/f", 0o600);
    let (_stdout, stderr, rc) = run_rubash(&dir.0, "chmod 600 d");
    assert_eq!(rc, Some(0), "rc (stderr {stderr:?})");
    assert_eq!(stderr, "");
    assert_mode(&dir.0, "d", 0o600);
    // Restore the directory so the scratch teardown can remove it.
    let (_stdout, stderr, rc) = run_rubash(&dir.0, "chmod 755 d");
    assert_eq!(rc, Some(0), "rc (stderr {stderr:?})");
    assert_eq!(stderr, "");
}

#[test]
fn chmod_symbolic_mode_through_file_reports_enotdir() {
    let dir = scratch("symbolic");
    // The symbolic clause reads its base mode from the node's stat;
    // the failure is still the walk's ENOTDIR, not the base fallback.
    assert_shape(
        &dir.0,
        "chmod +x a/b",
        1,
        "chmod: cannot access 'a/b': Not a directory\n",
    );
}

#[test]
fn chmod_file_trailing_slash_reports_enotdir() {
    let dir = scratch("slash");
    assert_shape(
        &dir.0,
        "chmod 600 a/",
        1,
        "chmod: cannot access 'a/': Not a directory\n",
    );
}
