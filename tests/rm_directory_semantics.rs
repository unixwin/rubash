//! niubash D2 regression: the `rm` emulation in
//! `src/executor/external_file_builtins.rs` used to walk any directory
//! operand with `remove_dir_all`, so `rm dir` deleted a non-empty tree and
//! exited 0 — GNU rm refuses a directory unless -r/-R or -d is given.
//! The same wrapper swallowed every failure under -f, so a locked file
//! reported success (niubash D3). These assertions pin the GNU outcome
//! table for both shapes.

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

fn run_in_scratch(script: &str, tag: &str) -> (Output, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rubash-rm-dir-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("tree").join("inner")).expect("scratch tree");
    std::fs::write(dir.join("tree").join("inner").join("f"), b"x").expect("fixture file");
    std::fs::create_dir_all(dir.join("empty")).expect("scratch empty dir");
    std::fs::write(dir.join("plain"), b"x").expect("scratch plain file");

    let output = Command::new(shell_bin())
        .args(["-c", script])
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    (
        Output {
            status: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).replace('\r', ""),
        },
        dir,
    )
}

fn clean(dir: PathBuf) {
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rm_non_empty_directory_without_recursive_flag_fails_and_survives() {
    let (out, dir) = run_in_scratch("rm tree", "nonempty");
    let survived = dir.join("tree").join("inner").join("f").exists();
    clean(dir);
    assert_eq!(
        out.status, 1,
        "stdout={:?} stderr={:?}",
        out.stdout, out.stderr
    );
    assert!(
        out.stderr.contains("Is a directory"),
        "stderr={:?}",
        out.stderr
    );
    assert!(survived, "the tree was deleted: stderr={:?}", out.stderr);
}

#[test]
fn rm_empty_directory_without_flag_fails() {
    let (out, dir) = run_in_scratch("rm empty", "empty");
    let survived = dir.join("empty").exists();
    clean(dir);
    assert_eq!(out.status, 1, "stderr={:?}", out.stderr);
    assert!(
        out.stderr.contains("Is a directory"),
        "stderr={:?}",
        out.stderr
    );
    assert!(survived, "the empty directory was removed");
}

#[test]
fn rm_force_does_not_authorize_directory_removal() {
    let (out, dir) = run_in_scratch("rm -f tree", "force-dir");
    let survived = dir.join("tree").exists();
    clean(dir);
    assert_eq!(out.status, 1, "stderr={:?}", out.stderr);
    assert!(
        out.stderr.contains("Is a directory"),
        "stderr={:?}",
        out.stderr
    );
    assert!(survived, "-f removed a directory operand");
}

#[test]
fn rm_recursive_removes_tree() {
    let (out, dir) = run_in_scratch("rm -r tree", "recursive");
    let survived = dir.join("tree").exists();
    clean(dir);
    assert_eq!(out.status, 0, "stderr={:?}", out.stderr);
    assert!(
        !survived,
        "-r left the tree behind: stderr={:?}",
        out.stderr
    );
}

#[test]
fn rm_capital_r_removes_tree() {
    let (out, dir) = run_in_scratch("rm -R tree", "recursive-caps");
    let survived = dir.join("tree").exists();
    clean(dir);
    assert_eq!(out.status, 0, "stderr={:?}", out.stderr);
    assert!(
        !survived,
        "-R left the tree behind: stderr={:?}",
        out.stderr
    );
}

#[test]
fn rm_dir_flag_removes_only_empty_directory() {
    let (out, dir) = run_in_scratch("rm -d empty", "dir-flag");
    let empty_gone = !dir.join("empty").exists();
    let tree_kept = dir.join("tree").join("inner").join("f").exists();
    clean(dir);
    assert_eq!(out.status, 0, "stderr={:?}", out.stderr);
    assert!(
        empty_gone,
        "-d left the empty directory: stderr={:?}",
        out.stderr
    );
    assert!(tree_kept, "the fixture tree vanished unexpectedly");
}

#[test]
fn rm_dir_flag_rejects_non_empty_directory() {
    let (out, dir) = run_in_scratch("rm -d tree", "dir-flag-nonempty");
    let survived = dir.join("tree").join("inner").join("f").exists();
    clean(dir);
    assert_eq!(out.status, 1, "stderr={:?}", out.stderr);
    assert!(
        out.stderr.contains("Directory not empty"),
        "stderr={:?}",
        out.stderr
    );
    assert!(survived, "-d emptied a non-empty tree");
}

#[test]
fn rm_missing_operand_is_an_error() {
    let (out, dir) = run_in_scratch("rm", "no-operand");
    clean(dir);
    assert_eq!(out.status, 1, "stderr={:?}", out.stderr);
    assert!(
        out.stderr.contains("missing operand"),
        "stderr={:?}",
        out.stderr
    );
}

#[test]
fn rm_force_still_reports_missing_operand() {
    let (out, dir) = run_in_scratch("rm -f", "no-operand-force");
    clean(dir);
    assert_eq!(out.status, 1, "stderr={:?}", out.stderr);
    assert!(
        out.stderr.contains("missing operand"),
        "stderr={:?}",
        out.stderr
    );
}

#[test]
fn rm_force_silences_only_nonexistent_operands() {
    let (out, dir) = run_in_scratch("rm -f missing plain", "force-missing");
    let plain_gone = !dir.join("plain").exists();
    clean(dir);
    assert_eq!(out.status, 0, "stderr={:?}", out.stderr);
    assert!(out.stderr.is_empty(), "stderr={:?}", out.stderr);
    assert!(plain_gone, "-f skipped the existing operand");
}

#[test]
fn rm_missing_operand_and_failing_operand_leave_status_one() {
    let (out, dir) = run_in_scratch("rm missing tree", "mixed");
    let survived = dir.join("tree").exists();
    clean(dir);
    assert_eq!(out.status, 1, "stderr={:?}", out.stderr);
    assert!(out.stderr.contains("missing"), "stderr={:?}", out.stderr);
    assert!(
        out.stderr.contains("Is a directory"),
        "stderr={:?}",
        out.stderr
    );
    assert!(survived, "rm deleted the directory operand");
}

#[test]
fn rm_double_dash_makes_dash_operands_literal() {
    let (out, dir) = run_in_scratch("rm -- tree", "double-dash");
    let survived = dir.join("tree").exists();
    clean(dir);
    assert_eq!(out.status, 1, "stderr={:?}", out.stderr);
    assert!(
        survived,
        "rm -- deleted a directory: stderr={:?}",
        out.stderr
    );
}

#[test]
fn rm_invalid_option_is_rejected_before_operands() {
    let (out, dir) = run_in_scratch("rm -Z plain", "bad-option");
    let plain_kept = dir.join("plain").exists();
    clean(dir);
    assert_eq!(out.status, 1, "stderr={:?}", out.stderr);
    assert!(
        out.stderr.contains("invalid option"),
        "stderr={:?}",
        out.stderr
    );
    assert!(plain_kept, "an invalid option still removed an operand");
}
