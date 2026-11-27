//! rubash#615 (W5 residual / characterization pin): the 24 Android
//! cli_tests reds were triaged into (A) bashdb fixture not bootstrapped,
//! (B) tests hardcoding Windows-environment outcomes, (C) host-tool /
//! platform-gated expectations, (D) two genuine candidates. The ruled
//! disposition is test-side: runtime-skip / fixture gate /
//! platform-conditional expectations — NOT forcing one host's outcomes
//! onto every platform.
//!
//! Face (A) is already pinned by tests/issue615_bashdb_fixture_gate.rs
//! (the `launcher_available` gate semantics). This file pins face (B)'s
//! canonical example, the `nUl` null-device alias, in its
//! platform-conditional form so the split itself cannot regress back
//! to a single hardcoded expectation:
//! - non-Windows: rubash matches GNU bash exactly — `nUl` is an ordinary
//!   missing file, redirect fails rc=1 (the issue's own manual repro
//!   shows both shells at rc=1 with the same diagnostic shape);
//! - Windows: `nUl` is the case-insensitive NUL device alias, rc=0.
//!
//! Face (C)'s CRLF example is GNU-parity on Android (both shells reject)
//! and desktop acceptance is pinned by the cli_tests
//! `*_accepts_crlf_control_flow` tests; no separate pin needed here.

use std::path::PathBuf;
use std::process::Command;

fn scratch_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-615-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn run_in_dir(script: &str, cwd: &std::path::Path) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .current_dir(cwd)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[cfg(not(windows))]
#[test]
fn issue615_nul_alias_is_gnu_parity_off_windows() {
    let dir = scratch_dir();
    let (stdout, stderr, status) = run_in_dir("cat < nUl; echo \"rc=$?\"", &dir);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr:?}");
    assert!(
        stderr.contains("nUl"),
        "diagnostic should name the operand, got: {stderr:?}"
    );
    assert_eq!(status, Some(0), "stderr: {stderr:?}");
}

#[cfg(windows)]
#[test]
fn issue615_nul_alias_resolves_on_windows() {
    let dir = scratch_dir();
    let (stdout, stderr, status) = run_in_dir("cat < nUl; echo \"rc=$?\"", &dir);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stdout, "rc=0\n", "stderr: {stderr:?}");
    assert_eq!(status, Some(0), "stderr: {stderr:?}");
}
