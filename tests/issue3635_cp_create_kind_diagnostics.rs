//! rubash#3635 — the emulated `cp`'s failed-create diagnostic names
//! the node kind, matching GNU coreutils copy.c copy_internal:
//! `cannot create regular file '<dst>'` for file targets and
//! `cannot create directory '<dst>'` for the recursive-tree branch.
//!
//! Before this fix both faces rendered a bare `cannot create '<dst>'`,
//! dropping the kind word — the builtins.tests
//! `cp /dev/null /zero-length-file-$$` residual in the run-builtins
//! lane (#2666). The tree walk itself also collapsed every nested
//! failure into one diagnostic naming the TOP target with the kind
//! picked from the top-level source — a failed `cp -r` child printed
//! `cannot create directory '<top>'` even when a regular file is what
//! failed, and the walk stopped at the first error where GNU's
//! copy_dir (`ok &= copy_internal (...)`) continues with the
//! siblings.
//!
//! Oracle (GNU bash 5.3.20 + coreutils 9.4, copy.c copy_internal /
//! copy_dir):
//!   cp /dev/null /zero-length-file-x
//!     -> cp: cannot create regular file '/zero-length-file-x':
//!        Permission denied   (rc 1)
//!   cp -r /tmp/…/a /zero-dir-x
//!     -> cp: cannot create directory '/zero-dir-x':
//!        Permission denied   (rc 1)
//!   cp -r src dst  (dst/src/sub a file, src/sub a directory)
//!     -> cp: cannot overwrite non-directory 'dst/src/sub'
//!        with directory 'src/sub'   (rc 1, siblings still copied)
//!   cp -r src dst  (nested file create denied)
//!     -> cp: cannot create regular file 'dst/src/blk/f':
//!        Permission denied   (rc 1, siblings still copied)

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn scratch_dir(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("rubash-3635-{tag}-{}", std::process::id()))
}

fn run_rubash_in(dir: &Path, script: &str) -> (String, String, Option<i32>) {
    std::fs::create_dir_all(dir).expect("create scratch dir");
    let script_path = dir.join("script.sh");
    std::fs::write(&script_path, script).expect("write script");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.current_dir(dir)
        .arg(&script_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match child.try_wait().expect("poll rubash") {
            Some(_) => break,
            None => {
                assert!(Instant::now() < deadline, "rubash timed out: {script}");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
    let output = child.wait_with_output().expect("collect rubash output");
    let _ = std::fs::remove_dir_all(dir);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code(),
    )
}

fn run_rubash(tag: &str, script: &str) -> (String, String, Option<i32>) {
    run_rubash_in(&scratch_dir(tag), script)
}

#[test]
fn cp_regular_file_create_failure_names_regular_file() {
    let (stdout, stderr, rc) = run_rubash("regular-file", "cp /dev/null /zero-length-file-x3635\n");
    assert_eq!(stdout, "");
    assert_eq!(rc, Some(1));
    assert_eq!(
        stderr,
        "cp: cannot create regular file '/zero-length-file-x3635': Permission denied\n"
    );
}

#[test]
fn cp_directory_create_failure_names_directory() {
    let (stdout, stderr, rc) = run_rubash(
        "directory",
        "mkdir -p /tmp/rb3635-src/a\ncp -r /tmp/rb3635-src/a /zero-dir-x3635\n",
    );
    assert_eq!(stdout, "");
    assert_eq!(rc, Some(1));
    assert_eq!(
        stderr,
        "cp: cannot create directory '/zero-dir-x3635': Permission denied\n"
    );
}

/// GNU copy.c copy_dir: a failed child is diagnosed at ITS node —
/// the kind and the path of what actually failed — and the walk
/// continues with the remaining siblings. Absolute operands keep the
/// display spelling identical to GNU's operand spelling.
#[test]
fn cp_nested_dir_over_file_names_node_and_continues() {
    let dir = scratch_dir("nested-dir-over-file");
    let d = dir.display().to_string();
    let script = format!(
        "mkdir -p {d}/srcdir/sub {d}/dst/srcdir\n\
         printf x > {d}/dst/srcdir/sub\n\
         printf y > {d}/srcdir/top\n\
         cp -r {d}/srcdir {d}/dst\n\
         cp_rc=$?\n\
         test -f {d}/dst/srcdir/top && echo sibling-ok || echo sibling-missing\n\
         exit $cp_rc\n"
    );
    let (stdout, stderr, rc) = run_rubash_in(&dir, &script);
    assert_eq!(rc, Some(1));
    assert_eq!(stdout, "sibling-ok\n");
    assert_eq!(
        stderr,
        format!(
            "cp: cannot overwrite non-directory '{d}/dst/srcdir/sub' with directory '{d}/srcdir/sub'\n"
        )
    );
}

/// The mirror gate: a file source never lands on an existing
/// directory (copy.c copy_internal, errno-less diagnostic).
#[test]
fn cp_nested_file_over_dir_names_node_and_continues() {
    let dir = scratch_dir("nested-file-over-dir");
    let d = dir.display().to_string();
    let script = format!(
        "mkdir -p {d}/srcdir {d}/dst/srcdir/adir\n\
         printf data > {d}/srcdir/adir\n\
         printf other > {d}/srcdir/other\n\
         cp -r {d}/srcdir {d}/dst\n\
         cp_rc=$?\n\
         test -f {d}/dst/srcdir/other && echo sibling-ok || echo sibling-missing\n\
         exit $cp_rc\n"
    );
    let (stdout, stderr, rc) = run_rubash_in(&dir, &script);
    assert_eq!(rc, Some(1));
    assert_eq!(stdout, "sibling-ok\n");
    assert_eq!(
        stderr,
        format!("cp: cannot overwrite directory '{d}/dst/srcdir/adir' with non-directory\n")
    );
}

/// A nested regular-file create failure keeps the `regular file`
/// kind AND the failing node's own path — the pre-fix walk reported
/// `cannot create directory '<top>'` for this shape.
#[test]
fn cp_nested_regular_file_create_failure_names_node_and_continues() {
    let dir = scratch_dir("nested-regular-file");
    let d = dir.display().to_string();
    let script = format!(
        "mkdir -p {d}/srcdir/blk {d}/dst/srcdir/blk\n\
         printf new > {d}/srcdir/blk/f\n\
         printf sib > {d}/srcdir/top\n\
         chmod 555 {d}/dst/srcdir/blk\n\
         cp -r {d}/srcdir {d}/dst\n\
         cp_rc=$?\n\
         chmod 755 {d}/dst/srcdir/blk\n\
         test -f {d}/dst/srcdir/top && echo sibling-ok || echo sibling-missing\n\
         exit $cp_rc\n"
    );
    let (stdout, stderr, rc) = run_rubash_in(&dir, &script);
    assert_eq!(rc, Some(1));
    assert_eq!(stdout, "sibling-ok\n");
    assert_eq!(
        stderr,
        format!("cp: cannot create regular file '{d}/dst/srcdir/blk/f': Permission denied\n")
    );
}

/// copy.c's mkdirat is single-level: a missing intermediate
/// component is ENOENT — the pre-fix create_dir_all silently built
/// the whole chain where GNU fails.
#[test]
fn cp_tree_missing_parent_names_directory() {
    let dir = scratch_dir("tree-missing-parent");
    let d = dir.display().to_string();
    let script = format!(
        "mkdir -p {d}/srcdir\nprintf x > {d}/srcdir/f\n\
         cp -r {d}/srcdir {d}/nodir-xyz/d\n"
    );
    let (stdout, stderr, rc) = run_rubash_in(&dir, &script);
    assert_eq!(stdout, "");
    assert_eq!(rc, Some(1));
    assert_eq!(
        stderr,
        format!("cp: cannot create directory '{d}/nodir-xyz/d': No such file or directory\n")
    );
}

/// dst_sb is lstat'd in copy_internal: a destination symlink — even
/// one resolving to a directory — is a non-directory for the
/// overwrite gate.
#[test]
fn cp_tree_symlink_destination_is_non_directory() {
    let dir = scratch_dir("tree-symlink-dst");
    let d = dir.display().to_string();
    let script = format!(
        "mkdir -p {d}/srcdir {d}/realdir {d}/dst\n\
         ln -s {d}/realdir {d}/dst/srcdir\n\
         cp -r {d}/srcdir {d}/dst\n"
    );
    let (stdout, stderr, rc) = run_rubash_in(&dir, &script);
    assert_eq!(stdout, "");
    assert_eq!(rc, Some(1));
    assert_eq!(
        stderr,
        format!(
            "cp: cannot overwrite non-directory '{d}/dst/srcdir' with directory '{d}/srcdir'\n"
        )
    );
}
