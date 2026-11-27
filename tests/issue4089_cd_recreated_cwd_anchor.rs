//! rubash#4089 — `cd` adoption of a deleted-and-recreated cwd is governed by
//! the EFFECTIVE chdir operand, not by chdir success alone.
//!
//! GNU Bash 5.3.20 oracle (script-file probe, 2026-11-27):
//!
//! ```text
//! S1 explicit `cd "$HOME/a"`      -> after=0 (adopts the recreated directory)
//! S2 poisoned logical `cd .`      -> after=1 (stays in the dead inode)
//! S3 bare logical `cd .`          -> after=0 (adopts the recreated directory)
//! S4 physical `cd -P .`           -> after=1 (stays in the dead inode)
//! S5 failed `cd`                  -> after=1
//! S6 no second `cd`               -> after=1
//! ```
//!
//! The split comes from builtins/cd.def change_to_directory: logical mode
//! makes the operand absolute against the_current_working_directory and
//! chdirs the canonical path when canon succeeds (cd.def:535-542, 600), so
//! S1/S3 re-enter the live path. A poisoned entry cannot restore that base
//! in a dead cwd (cd.def:529-535; builtins/common.c:592-618), so `.` stays a
//! relative effective operand (general.c:885-902); physical mode chdirs the
//! verbatim operand first (cd.def:600), so `cd -P .` likewise stays in the
//! current inode.
//!
//! Unix-only: the inode-anchor behavior being pinned is the Unix cwd anchor.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

struct Tree {
    base: PathBuf,
    real: String,
}

fn make_tree(tag: &str) -> Tree {
    let base = std::env::temp_dir().join(format!("rubash4089-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("a")).expect("create a");
    let real = std::fs::canonicalize(&base)
        .expect("canonicalize base")
        .to_string_lossy()
        .into_owned();
    Tree { base, real }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn run_script_in(dir: &std::path::Path, script: &str) -> (String, String, Option<i32>) {
    let case = dir.join("case.sh");
    std::fs::write(&case, script).expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(dir)
        .arg(&case)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_file(&case);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Stage the deleted-and-recreated cwd, run `shape`, then ask an external
/// `/bin/cat` child whether it can see the file that exists only in the
/// recreated directory. `before_recreate` is used by the poisoned shape.
fn run_shape(tag: &str, before_recreate: &str, shape: &str) -> (String, String, Option<i32>) {
    let tree = make_tree(tag);
    let script = format!(
        "cd \"{r}/a\" || exit 99\n\
         (cd ..; rmdir a) || exit 98\n\
         {before_recreate}\
         mkdir \"{r}/a\" || exit 97\n\
         printf 'ghost\\n' > \"{r}/a/marker\" || exit 96\n\
         HOME=\"{r}\"\n\
         {shape}\
         /bin/cat marker >/dev/null 2>&1; echo after=$?\n",
        r = tree.real,
        before_recreate = before_recreate,
        shape = shape
    );
    run_script_in(&tree.base, &script)
}

fn assert_shape(tag: &str, before_recreate: &str, shape: &str, expected_stdout: &str) {
    let (stdout, stderr, code) = run_shape(tag, before_recreate, shape);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stderr, "");
    assert_eq!(stdout, expected_stdout);
}

#[test]
fn issue4089_s1_explicit_cd_home_adopts_recreated_dir() {
    assert_shape(
        "s1",
        "",
        "cd \"$HOME/a\" >/dev/null 2>&1; echo cdrc=$?\n",
        "cdrc=0\nafter=0\n",
    );
}

#[test]
fn issue4089_s2_poisoned_logical_dot_stays_on_dead_inode() {
    assert_shape(
        "s2",
        "pwd -P >/dev/null 2>&1; echo poisonrc=$?\n",
        "cd . >/dev/null 2>&1; echo cdrc=$?\n",
        "poisonrc=1\ncdrc=0\nafter=1\n",
    );
}

#[test]
fn issue4089_s3_bare_logical_dot_adopts_recreated_dir() {
    assert_shape(
        "s3",
        "",
        "cd . >/dev/null 2>&1; echo cdrc=$?\n",
        "cdrc=0\nafter=0\n",
    );
}

#[test]
fn issue4089_s4_physical_dot_stays_on_dead_inode() {
    assert_shape(
        "s4",
        "",
        "cd -P . >/dev/null 2>&1; echo cdrc=$?\n",
        "cdrc=0\nafter=1\n",
    );
}

#[test]
fn issue4089_s5_failed_cd_does_not_adopt_recreated_dir() {
    assert_shape(
        "s5",
        "",
        "cd \"$HOME/nope\" >/dev/null 2>&1; echo cdrc=$?\n",
        "cdrc=1\nafter=1\n",
    );
}

#[test]
fn issue4089_s6_without_second_cd_ghost_read_stays_closed() {
    assert_shape("s6", "", "", "after=1\n");
}
