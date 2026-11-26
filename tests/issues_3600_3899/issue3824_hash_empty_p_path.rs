//! rubash#3824 — `hash -p ''` stores an empty path; GNU keeps it empty.
//!
//! GNU anchors (bash 5.3):
//! - `hashcmd.c` `phash_search`: a RELPATH-flagged entry is tested with
//!   `"./"` prepended, and the test is GNU `executable_file`
//!   (`findcmd.c:214-223`), which requires FS_EXECABLE **and** not a
//!   directory. For the stored path `""` the dotted form `"./"` is a
//!   directory, so the entry is not executable and `phash_search`
//!   returns the stored path as-is — `""`.
//! - `hash.def:163-180` `add_hashed_command`: the `-p` insert stores the
//!   given path verbatim (only `.`/`/` are rejected as directories);
//!   the divergence is at READ time, not at insert time.
//!
//! rubash's unix `is_executable_file` (`src/executor/path.rs`) is a
//! bare `access(X_OK)`, which admits directories (search permission),
//! so `phash_search_spelling` returns `"./"` and running the entry
//! fails `"./: Is a directory"` (rc 126) instead of GNU's
//! `": No such file or directory"` (rc 127).
//!
//! Expectations are GNU bash 5.3.20 behaviour for the verbatim seed
//! scripts (R133-A), run from a file in a fresh directory. The defect
//! is unix-only: the Windows `is_executable_file` arm is `is_file()`,
//! which already excludes directories.
//!
#![cfg(unix)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("rubash-3824-{tag}-{pid}", pid = std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch dir");
        Scratch(dir)
    }

    fn rubash_file(&self, script: &str) -> (String, String, Option<i32>) {
        let file = self.0.join("case.sh");
        fs::write(&file, script).expect("write script");
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&file)
            .current_dir(&self.0)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.0)
            .output()
            .expect("run rubash");
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
        )
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn issue3824_hash_t_prints_empty_path_for_empty_p_entry() {
    let scratch = Scratch::new("t");
    let (stdout, stderr, code) = scratch.rubash_file("hash -p '' ghost\nhash -t ghost\n");
    assert_eq!(stdout, "\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3824_running_empty_p_entry_is_no_such_file_rc127() {
    let scratch = Scratch::new("run");
    let (stdout, stderr, code) = scratch.rubash_file("hash -p '' ghost\nghost; echo rc=$?\n");
    assert_eq!(stdout, "rc=127\n");
    assert!(
        stderr.contains(": No such file or directory"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue3824_hash_l_prints_empty_path_for_empty_p_entry() {
    let scratch = Scratch::new("l");
    let (stdout, stderr, code) = scratch.rubash_file("hash -p '' ghost\nhash -l\n");
    assert_eq!(stdout, "builtin hash -p  ghost\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3824_hash_table_lists_empty_path_for_empty_p_entry() {
    let scratch = Scratch::new("table");
    let (stdout, stderr, code) = scratch.rubash_file("hash -p '' ghost\nhash\n");
    assert_eq!(stdout, "hits\tcommand\n   0\t\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3824_hash_p_dot_is_rejected_as_directory() {
    let scratch = Scratch::new("dot");
    let (stdout, stderr, code) =
        scratch.rubash_file("hash -p . ghost; echo rc=$?\nhash -t ghost; echo rc2=$?\n");
    assert_eq!(stdout, "rc=1\nrc2=1\n");
    assert!(
        stderr.contains("hash: .: Is a directory"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("hash: ghost: not found"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue3824_hash_p_slash_is_rejected_as_directory() {
    let scratch = Scratch::new("slash");
    let (stdout, stderr, code) =
        scratch.rubash_file("hash -p / ghost; echo rc=$?\nhash -t ghost; echo rc2=$?\n");
    assert_eq!(stdout, "rc=1\nrc2=1\n");
    assert!(
        stderr.contains("hash: /: Is a directory"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("hash: ghost: not found"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue3824_running_rejected_dot_entry_is_command_not_found() {
    let scratch = Scratch::new("dotrun");
    let (stdout, stderr, code) =
        scratch.rubash_file("hash -p . ghost 2>/dev/null\nghost; echo rc=$?\n");
    assert_eq!(stdout, "rc=127\n");
    assert!(
        stderr.contains("ghost: command not found"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue3824_hash_t_prints_empty_path_after_double_empty_p() {
    let scratch = Scratch::new("twice");
    let (stdout, stderr, code) =
        scratch.rubash_file("hash -p '' ghost\nhash -p '' ghost\nhash -t ghost\n");
    assert_eq!(stdout, "\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
