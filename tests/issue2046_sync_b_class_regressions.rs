//! rubash#2046 (S5 of the rubash#2036 sync line): B-class pins.
//!
//! B1 — the comsub basename arm was the projection's Unchecked cell:
//! it answered `$(basename /a/b/c)` = "c" even with PATH holding no
//! basename at all, where GNU (subst.c:7143 command_substitute →
//! full execution → PATH lookup) fails 127. S3 adjudicated the cell
//! FailClosed (DECISIONS 2026-10-29); S5 adds the same stand-down
//! predicate the sibling cat arm already consults (hash record
//! first, PATH walk only on a miss — no measurable cost on the
//! comsub hot shape). The remaining #2046 cells were either ruled
//! keep (documented in the S5 corpus, which pins their shapes) or
//! are unruled and registered-only there — this file pins B1 and its
//! face contrasts.

use std::process::Command;

fn rubash() -> std::path::PathBuf {
    let mut path = std::env::current_exe().expect("current exe");
    path.pop();
    path.pop();
    path.push("rubash");
    if cfg!(windows) {
        path.set_extension("exe");
    }
    path
}

struct Output {
    stdout: String,
    stderr: String,
}

fn run(script: &str, dir: &std::path::Path, path: &str) -> Output {
    let output = Command::new(rubash())
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .env("PATH", path)
        .output()
        .expect("spawn rubash");
    Output {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-2046-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

#[test]
fn issue2046_b1_comsub_basename_fails_closed_without_path_basename() {
    let dir = scratch("b1");
    let out = run(
        "x=$(basename /a/b/c.txt); echo \"rc=$? [$x]\"",
        &dir,
        "/nonexistent",
    );
    assert_eq!(out.stdout, "rc=127 []\n", "GNU: empty value, status 127");
    assert!(
        out.stderr.contains("basename: command not found"),
        "the lookup failure must be diagnosed, got {:?}",
        out.stderr
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue2046_b1_contrasts() {
    let dir = scratch("b1c");
    // With a PATH basename present the arm still answers in-process.
    let out = run(
        "x=$(basename /a/b/c.txt); echo \"rc=$? [$x]\"",
        &dir,
        "/usr/bin:/bin",
    );
    assert_eq!(out.stdout, "rc=0 [c.txt]\n");
    // The simple face already stood down before S5 — same answer.
    let out = run("basename /a/b/c; echo rc=$?", &dir, "/nonexistent");
    assert_eq!(out.stdout, "rc=127\n");
    // The comsub dirname shape (no arm) was already FailClosed —
    // the two comsub faces now agree.
    let out = run(
        "x=$(dirname /a/b/c); echo \"rc=$? [$x]\"",
        &dir,
        "/nonexistent",
    );
    assert_eq!(out.stdout, "rc=127 []\n");
    let _ = std::fs::remove_dir_all(&dir);
}
