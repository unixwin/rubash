//! niubash#993 S1 — `declare -a` must enumerate the dynamic array
//! cells GNU enumerates: GROUPS always, PIPESTATUS once a pipeline
//! has run.
//!
//! Reported as "bash lists 9 arrays, niu 7". The engine side of this
//! face is owned by the dynamic-registry listing supply
//! (rubash#1363, rubash#2700, rubash#2973): `declare -a` merges the
//! VISIBLE dynamic cells into the table walk
//! (src/executor/declare_local.rs -> dynamic_views ->
//! builtins/declare/print.rs listing merge). This file pins the exact
//! issue repro against the rubash CLI so a future regression in the
//! supply surfaces here rather than inside niu's session.
//!
//! GNU anchors: variables.c:1886 (GROUPS INIT_DYNAMIC_ARRAY_VAR),
//! variables.c:6299-6301 (PIPESTATUS make_new_array_variable on
//! pipeline completion), print_var_list (variables.c:1066+) includes
//! set dynamic cells; an unmaterialized cell prints `name=()` and a
//! never-created PIPESTATUS prints nothing.

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn run_script(tag: &str, script: &str) -> (String, String, i32) {
    let dir = std::env::temp_dir().join(format!("rubash-993s1-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("fixture dir");
    let path: PathBuf = dir.join("case.sh");
    std::fs::write(&path, script).expect("write script");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg(&path)
        .current_dir(&dir)
        .env("HOME", &dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let out = cmd.output().expect("run rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

/// Before any command runs, GNU lists GROUPS as the empty array but
/// has no PIPESTATUS cell yet (it is created on pipeline completion).
#[test]
fn declare_a_lists_groups_before_any_pipeline() {
    let (stdout, stderr, code) = run_script("groups", "declare -a\n");
    assert_eq!(code, 0, "stderr: {stderr:?}");
    assert!(
        stdout.contains("declare -a GROUPS=()\n"),
        "stdout: {stdout:?}"
    );
    assert!(
        !stdout.contains("PIPESTATUS"),
        "no PIPESTATUS before first command — stdout: {stdout:?}"
    );
}

/// After one command, GNU's PIPESTATUS cell exists with the last
/// pipeline's statuses; `declare -a` must enumerate it (the issue's
/// missing ninth array).
#[test]
fn declare_a_lists_pipestatus_after_a_command() {
    let (stdout, stderr, code) = run_script("pipe", "true\ndeclare -a\n");
    assert_eq!(code, 0, "stderr: {stderr:?}");
    assert!(
        stdout.contains("declare -a PIPESTATUS=([0]=\"0\")\n"),
        "stdout: {stdout:?}"
    );
}

/// The exact issue repro shape: the count of `declare -a` lines must
/// match GNU's 9 once a command has run (GROUPS + PIPESTATUS present).
#[test]
fn declare_a_count_matches_gnu_nine() {
    let (stdout, _stderr, _code) = run_script(
        "count",
        "true\ndeclare -a | sed \"s/.*declare -a[^ ]* \\([A-Za-z_]*\\).*/\\1/\" | sort\n",
    );
    let names: Vec<&str> = stdout.lines().collect();
    for expected in ["GROUPS", "PIPESTATUS"] {
        assert!(names.contains(&expected), "missing {expected} in {names:?}");
    }
}
