//! rubash#1665 — no-output builtins (`true`, `:`, `false`) skipped the
//! file-creation side effect of `N>file` redirections on fd >= 3: their
//! redirect path (`apply_no_output_builtin_redirects`) mirrored only the
//! fd 0/1/2 fields and never walked the fd >= 3 redirect entries, so
//! `true 3>fa` created nothing and a failing fd >= 3 open was silent.
//!
//! GNU redir.c do_redirections (redir.c:260 do_redirection_internal)
//! runs for EVERY simple command before the command body: each
//! redirection is opened in list order, creating/truncating its target
//! even when the command writes nothing, and a failed open is a
//! redirection_error (redir.c:149-176) — the command does not run and
//! the status is 1. Side effects of redirections earlier in the list
//! stand; later ones never happen.
//!
//! Expectations hand-measured against GNU bash 5.3.20 via script files.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir =
        std::env::temp_dir().join(format!("rubash-issue1665-{}-{tag}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn run_c_in(dir: &std::path::Path, script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(dir)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue1665_true_fd3_redirect_creates_file() {
    let dir = scratch("true-fd3");
    let (_stdout, _stderr, code) = run_c_in(&dir, "true 3>fa");
    assert_eq!(code, Some(0));
    let fa = dir.join("fa");
    assert!(fa.exists(), "true 3>fa must create fa");
    assert_eq!(std::fs::read(&fa).expect("read fa").len(), 0);
}

#[test]
fn issue1665_colon_and_false_fd_redirects_create_files() {
    let dir = scratch("colon-false");
    let (stdout, _stderr, _code) = run_c_in(&dir, ": 3>fb; false 4>fc; echo \"rc=$?\"");
    // `false` keeps its own status 1, but the file must exist.
    assert_eq!(stdout, "rc=1\n");
    assert!(dir.join("fb").exists(), ": 3>fb must create fb");
    assert!(dir.join("fc").exists(), "false 4>fc must create fc");
}

#[test]
fn issue1665_multiple_numbered_redirects_all_created() {
    let dir = scratch("multi");
    let (_stdout, _stderr, code) = run_c_in(&dir, "true 3>fa 4>fb");
    assert_eq!(code, Some(0));
    assert!(dir.join("fa").exists(), "true 3>fa 4>fb must create fa");
    assert!(dir.join("fb").exists(), "true 3>fa 4>fb must create fb");
}

#[test]
fn issue1665_fd3_redirect_truncates_and_appends() {
    let dir = scratch("trunc-append");
    std::fs::write(dir.join("fa"), "seed\n").expect("seed fa");
    std::fs::write(dir.join("fe"), "seed\n").expect("seed fe");
    let (_stdout, _stderr, code) = run_c_in(&dir, "true 3>fa; true 3>>fe");
    assert_eq!(code, Some(0));
    assert_eq!(std::fs::read(dir.join("fa")).expect("read fa").len(), 0);
    assert_eq!(
        std::fs::read_to_string(dir.join("fe")).expect("read fe"),
        "seed\n",
        "3>> must append (content preserved), not truncate"
    );
}

#[test]
fn issue1665_fd3_open_failure_is_fatal_with_diagnostic() {
    let dir = scratch("fail");
    let (stdout, stderr, _code) = run_c_in(&dir, "true 3>sub/f; echo \"rc=$?\"");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("sub/f: No such file or directory"),
        "stderr must name the failed target, got: {stderr}"
    );
}

#[test]
fn issue1665_redirect_order_side_effects_before_failure_stand() {
    let dir = scratch("order-before");
    let (stdout, _stderr, _code) = run_c_in(&dir, "true 3>fa 3>sub/f; echo \"rc=$?\"");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        dir.join("fa").exists(),
        "the earlier 3>fa side effect stands when a later redirect fails"
    );
}

#[test]
fn issue1665_redirect_failure_aborts_later_redirects() {
    let dir = scratch("order-after");
    let (stdout, _stderr, _code) = run_c_in(&dir, "true 3>sub/f 3>fb; echo \"rc=$?\"");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        !dir.join("fb").exists(),
        "a failed redirect aborts the walk: 3>fb must not be opened"
    );
}

#[test]
fn issue1665_fd3_side_effect_before_failing_stdout_redirect() {
    let dir = scratch("order-mixed");
    let (stdout, _stderr, _code) = run_c_in(&dir, "true 3>fc 1>sub/f; echo \"rc=$?\"");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        dir.join("fc").exists(),
        "3>fc precedes the failing 1>sub/f in list order and must be created"
    );
}

#[test]
fn issue1665_input_failure_prevents_later_fd3_creation() {
    let dir = scratch("order-input");
    let (stdout, _stderr, _code) = run_c_in(&dir, "true <in-nothere 3>ff; echo \"rc=$?\"");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        !dir.join("ff").exists(),
        "the failing <in-nothere comes first: 3>ff must not be opened"
    );
}

#[test]
fn issue1665_dup_redirect_creates_no_file() {
    let dir = scratch("dup");
    let (_stdout, _stderr, code) = run_c_in(&dir, "true 3>&1");
    assert_eq!(code, Some(0));
    let entries: Vec<_> = std::fs::read_dir(&dir).expect("read scratch").collect();
    assert!(entries.is_empty(), "3>&1 must not create any file");
}

#[test]
fn issue1665_command_and_builtin_prefix_paths_create_files() {
    let dir = scratch("prefix");
    let (_stdout, _stderr, code) = run_c_in(&dir, "command true 3>fd; builtin true 3>fe");
    assert_eq!(code, Some(0));
    assert!(dir.join("fd").exists(), "command true 3>fd must create fd");
    assert!(dir.join("fe").exists(), "builtin true 3>fe must create fe");
}

#[test]
fn issue1665_command_prefix_fd3_failure_is_fatal() {
    let dir = scratch("prefix-fail");
    let (stdout, stderr, _code) = run_c_in(&dir, "command true 3>sub/f; echo \"rc=$?\"");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("sub/f: No such file or directory"),
        "stderr must name the failed target, got: {stderr}"
    );
}
