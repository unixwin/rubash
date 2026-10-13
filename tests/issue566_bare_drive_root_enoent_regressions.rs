//! Issue rubash#566: the BARE drive root — `cd /c` with no trailing slash
//! and no further component — failed with ENOENT whenever a shell root was
//! configured: `map_logical_path`'s drive-form exclusion required
//! `len() >= 3 && bytes[2] == b'/'`, so the two-letter spelling slipped past
//! the drive arm in `shell_path_to_windows_inner` and was resolved
//! root-relatively to `<root>/c` → "No such file or directory". `cd /c/`
//! always worked (its byte 2 IS the slash); a root that happened to carry a
//! `d/` directory made `cd /d` "work" by pure coincidence.
//!
//! Boundary contract B① (docs/windows-path-contract.md §L1) accepts the
//! drive-form operands `/c/...`, `/c/`, `/c/.`, `/D` (case-insensitive) —
//! the bare `/c` root is the same operand spelling family, and this file is
//! the lock the contract suite deliberately left to the issue (see the
//! "filed separately" note in path_contract_boundary_posix_forms.rs).
//!
//! Oracle: the contract text (authoritative audit 2026-10-12 against
//! master eb062957), plus live probes — the in-shell POSIX-render invariants
//! (`$PWD == $(pwd)`, `/<drive>/…` form) come from the B① rows, not from
//! GNU (a GNU/Linux host has no drive-root concept; `cd /c` there is an
//! ordinary missing directory).
//!
//! Anchors: src/executor/path.rs `shell_path_to_windows_inner` drive arm
//! (the bare-root early return) and `map_logical_path` (the completed
//! drive-form exclusion).

#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn run_rubash(script: &str, extra_env: &[(&str, &str)]) -> (Option<i32>, String, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command.args(["-c", script]);
    for (name, value) in extra_env {
        command.env(name, value);
    }
    let output = command.output().expect("spawn rubash");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// A shell root WITHOUT a `c` child — the exact #566 repro shape: the old
/// root-relative mapping could not satisfy `cd /c` and failed with ENOENT.
fn bare_root_without_c(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash566-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A shell root WITH a `c` child carrying a marker: `cd /c` must reach the
/// DRIVE root (pwd `/c/`), never the same-named root child.
fn root_with_c_child(tag: &str) -> PathBuf {
    let dir = bare_root_without_c(tag);
    std::fs::create_dir_all(dir.join("c")).unwrap();
    std::fs::write(dir.join("c").join("marker.txt"), b"root child").unwrap();
    dir
}

/// The issue's shape: with a configured shell root lacking `c`, the bare
/// drive root `cd /c` succeeds and renders the POSIX drive form.
#[test]
fn bare_drive_root_cd_under_shell_root_without_c_child() {
    let root = bare_root_without_c("repro");
    let root_str = root.to_string_lossy().into_owned();
    let env = [("__RUBASH_SHELL_ROOT", root_str.as_str())];
    let (code, stdout, stderr) = run_rubash("cd /c && pwd", &env);
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(stdout, "/c/\n", "pwd must render the POSIX drive form");
    assert!(stderr.is_empty(), "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&root);
}

/// The old root-relative mapping is dead even when a same-named child
/// exists: `cd /c` reaches `C:\`, not `<root>/c` (the marker stays
/// invisible; the coincidence that made `cd /d` "work" is retired).
#[test]
fn bare_drive_root_wins_over_same_named_root_child() {
    let root = root_with_c_child("shadow");
    let root_str = root.to_string_lossy().into_owned();
    let env = [("__RUBASH_SHELL_ROOT", root_str.as_str())];
    let (code, stdout, stderr) = run_rubash(
        "cd /c && pwd; [[ -e marker.txt ]] && echo SHADOWED || echo drive_root",
        &env,
    );
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    assert!(stdout.contains("/c/"), "stdout: {stdout}");
    assert!(stdout.contains("drive_root"), "stdout: {stdout}");
    assert!(!stdout.contains("SHADOWED"), "stdout: {stdout}");
    let _ = std::fs::remove_dir_all(&root);
}

/// The slashed and dot siblings are byte-identical afterwards — the bare
/// root is now the same operand, not a third destination.
#[test]
fn slashed_and_dot_siblings_unchanged() {
    let root = bare_root_without_c("siblings");
    let root_str = root.to_string_lossy().into_owned();
    let env = [("__RUBASH_SHELL_ROOT", root_str.as_str())];
    for script in ["cd /c/ && pwd", "cd /c/. && pwd", "cd /c/./ && pwd"] {
        let (code, stdout, stderr) = run_rubash(script, &env);
        assert_eq!(code, Some(0), "script {script:?}: stderr: {stderr}");
        assert_eq!(stdout, "/c/\n", "script {script:?}");
        assert!(stderr.is_empty(), "script {script:?}: stderr: {stderr}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// `/D`-style uppercase spellings stay accepted (contract B① list): the
/// drive letter is case-insensitive on both sides.
#[test]
fn uppercase_drive_spelling_accepted() {
    let root = bare_root_without_c("upper");
    let root_str = root.to_string_lossy().into_owned();
    let env = [("__RUBASH_SHELL_ROOT", root_str.as_str())];
    let (code, stdout, stderr) = run_rubash("cd /C && pwd", &env);
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(stdout, "/c/\n");
    assert!(stderr.is_empty(), "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&root);
}

/// Boundary: ONLY the exact `/X` root spelling is a drive. `/cfoo` is an
/// ordinary (missing) root child and still fails like GNU's cd.
#[test]
fn non_drive_two_letter_prefix_stays_ordinary_path() {
    let root = bare_root_without_c("cfoo");
    let root_str = root.to_string_lossy().into_owned();
    let env = [("__RUBASH_SHELL_ROOT", root_str.as_str())];
    let (code, stdout, stderr) = run_rubash("cd /cfoo", &env);
    assert_eq!(code, Some(1), "stdout: {stdout} stderr: {stderr}");
    assert!(
        stderr.contains("No such file or directory"),
        "stderr: {stderr}"
    );
    assert!(stdout.is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

/// The B① render invariants hold after the bare-root cd: `$PWD == $(pwd)`
/// and the value is the POSIX drive form.
#[test]
fn pwd_render_invariants_after_bare_root_cd() {
    let root = bare_root_without_c("render");
    let root_str = root.to_string_lossy().into_owned();
    let env = [("__RUBASH_SHELL_ROOT", root_str.as_str())];
    let (code, stdout, stderr) = run_rubash(
        "cd /c || exit 9; [[ $PWD == $(pwd) ]] || exit 8; \
         [[ $PWD == /c/ ]] || exit 7; echo rendered=$PWD",
        &env,
    );
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(stdout, "rendered=/c/\n");
    assert!(stderr.is_empty(), "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&root);
}

/// Navigation through the bare root keeps working in both directions: the
/// shell can come back out of `C:\` to a deep POSIX operand.
#[test]
fn chained_navigation_through_bare_root() {
    let scratch = std::env::temp_dir().join(format!("rubash566-nav-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    let win = scratch.to_string_lossy().replace('\\', "/");
    let posix = format!(
        "/{}/{}",
        (win.as_bytes()[0] as char).to_ascii_lowercase(),
        &win[3..]
    );
    let root = bare_root_without_c("nav");
    let root_str = root.to_string_lossy().into_owned();
    let env = [("__RUBASH_SHELL_ROOT", root_str.as_str())];
    let (code, stdout, stderr) = run_rubash(&format!("cd /c && cd '{posix}' && pwd"), &env);
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(stdout, format!("{posix}\n"));
    assert!(stderr.is_empty(), "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&scratch);
}
