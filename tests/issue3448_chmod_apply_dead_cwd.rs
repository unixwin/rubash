//! rubash#3448 — the emulated `chmod`'s APPLY body under a dead
//! cwd. PR #3440 (Refs #3425) routed the operand stat through the
//! `CpNode` anchor, so the classification was right (`chmod 600
//! a/b` reports the walk's ENOTDIR), but the apply arm still went
//! through the joined string path: under a dead cwd
//! `metadata(&host)` / `set_permissions(&host)` die ENOENT on the
//! dead component, so `chmod 600 a` failed and the mode never
//! landed even though the node stat had just succeeded.
//!
//! The fix applies the mode through the node itself
//! (`CpNode::set_mode_bits_checked`: fchmodat against the anchor
//! for an At node, `std::fs::set_permissions` for a Plain node),
//! the same walk GNU's forked child gets — bash keeps no cwd
//! string, the coreutils child inherits the directory inode
//! (execute_cmd.c execute_disk_command).
//!
//! The dead cwd is staged the v3 way (reviewer probe
//! `probe-dead-cwd3.sh` in ~/workspace/review-3440/): the session
//! `cd`s into `t`, a subshell renames `t` to `t_gone` from the
//! parent, and the session stays in the renamed-away inode — the
//! files survive under the new name, so the anchor face still sees
//! them. (Deleting the cwd instead would unlink the operands too
//! and make ENOENT correct on both trees.)
//!
//! Oracle: GNU bash 5.3 + coreutils 9.4 (`chmod.c:231` — fts's
//! errno verbatim), v3 probe re-run on this lane 2026-11-13:
//!   chmod 600 a    (dead cwd) -> rc 0, silent, mode 600
//!   chmod u+x a    (dead cwd) -> rc 0, silent, mode 744 (from 644)
//!   chmod 600 d/f  (dead cwd) -> rc 0, silent, mode 600
//!   chmod 600 a/b  (dead cwd) -> rc 1, "cannot access 'a/b': Not a
//!                                directory", mode of a unchanged
//!   chmod 600 nope (dead cwd) -> rc 1, "cannot access 'nope': No
//!                                such file or directory"
//! Base rubash (master 3745712f) fails the first three cells: rc 1
//! ENOENT text, mode unchanged — the apply never reaches the file.
//!
//! These pins assert completion forms (stdout + stderr bytes +
//! exit status + physical modes), never timing. Unix only: the
//! cwd anchor is a Unix directory handle; off Unix there is no
//! anchor and the face keeps its pre-fix shape.

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

/// A scratch root holding `t/a` (regular file, mode 0644) and
/// `t/d/f` (a real directory with a member) — the directory the
/// shell enters and loses by rename.
fn scratch(case: &str) -> ScratchDir {
    let dir = std::env::temp_dir().join(format!("rubash-3448-{case}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("t/d")).expect("create t/d");
    std::fs::write(dir.join("t/a"), "A\n").expect("write t/a");
    std::fs::write(dir.join("t/d/f"), "DF\n").expect("write t/d/f");
    std::fs::set_permissions(dir.join("t/a"), std::fs::Permissions::from_mode(0o644))
        .expect("mode t/a");
    std::fs::set_permissions(dir.join("t/d/f"), std::fs::Permissions::from_mode(0o644))
        .expect("mode t/d/f");
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
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                panic!("rubash did not exit within 30s: {script}");
            }
            None => std::thread::sleep(Duration::from_millis(10)),
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

/// The dead-cwd prelude: enter `t`, have a subshell rename it away
/// from the parent, stay in the renamed-away inode; then `body`.
/// The session's own exit code is 0 throughout (the chmod verdict
/// is echoed), so the pins read it off stdout.
fn dead_script(body: &str) -> String {
    format!("cd t || exit 99\n(cd ..; mv t t_gone) || exit 98\n{body}\n")
}

fn assert_dead_shape(dir: &Path, body: &str, want_chmod_rc: i32, want_stderr: &str) {
    let script = dead_script(body);
    let (stdout, stderr, rc) = run_rubash(dir, &script);
    assert_eq!(rc, Some(0), "shell rc for {script:?} (stderr {stderr:?})");
    assert_eq!(
        stdout,
        format!("chmod_rc={want_chmod_rc}\n"),
        "stdout for {script:?}"
    );
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
fn chmod_apply_dead_cwd_lands() {
    let dir = scratch("apply");
    assert_dead_shape(&dir.0, "chmod 600 a; echo chmod_rc=$?", 0, "");
    assert_mode(&dir.0, "t_gone/a", 0o600);
}

#[test]
fn chmod_apply_dead_cwd_symbolic_lands() {
    let dir = scratch("symbolic");
    // u+x names its who, so no umask masking applies: 0644 -> 0744
    // whatever the runner's umask is.
    assert_dead_shape(&dir.0, "chmod u+x a; echo chmod_rc=$?", 0, "");
    assert_mode(&dir.0, "t_gone/a", 0o744);
}

#[test]
fn chmod_apply_dead_cwd_nested_lands() {
    let dir = scratch("nested");
    assert_dead_shape(&dir.0, "chmod 600 d/f; echo chmod_rc=$?", 0, "");
    assert_mode(&dir.0, "t_gone/d/f", 0o600);
}

#[test]
fn chmod_diag_dead_cwd_still_enotdir() {
    let dir = scratch("diag");
    assert_dead_shape(
        &dir.0,
        "chmod 600 a/b; echo chmod_rc=$?",
        1,
        "chmod: cannot access 'a/b': Not a directory\n",
    );
    assert_mode(&dir.0, "t_gone/a", 0o644);
}

#[test]
fn chmod_missing_dead_cwd_still_enoent() {
    let dir = scratch("missing");
    assert_dead_shape(
        &dir.0,
        "chmod 600 nope; echo chmod_rc=$?",
        1,
        "chmod: cannot access 'nope': No such file or directory\n",
    );
    assert_mode(&dir.0, "t_gone/a", 0o644);
}
