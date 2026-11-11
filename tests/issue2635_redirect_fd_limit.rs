//! rubash#2635 — a redirection whose target fd is at or above the
//! RLIMIT_NOFILE *soft* limit must fail `Bad file descriptor` with
//! status 1; rubash accepted every fd.
//!
//! GNU redir.c do_redirection_internal: the open succeeds (the
//! target file IS created), then the dup2 onto the redirector fd
//! fails EBADF when the redirector is at/above the soft limit
//! (redir.c r_output_direction / r_duplicating_output cases;
//! redirection_error at redir.c:149-176 renders
//! `<fd>: Bad file descriptor`). The failure lands at the entry's
//! own position in the list: earlier entries' side effects stand,
//! later entries never run. Close forms (`N>&-`) are exempt —
//! r_close_this ignores the close result. For a dup entry the
//! diagnostic names the SOURCE fd (the word being duplicated).
//!
//! Expectations byte-verified against GNU bash 5.3.20 with
//! script-file probes under `ulimit -n 16` (fds 9/10/15 pass;
//! 16/17/20 fail).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2635-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Run `body` as a script FILE under rubash with cwd = dir.
/// Returns (stdout, stderr).
fn rubash_script(dir: &Path, name: &str, body: &str) -> (String, String) {
    let script = dir.join(name);
    fs::write(&script, body).expect("write script fixture");
    let output = Command::new(RUBASH)
        .arg(&script)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn file_contents(dir: &Path, name: &str) -> Option<String> {
    fs::read_to_string(dir.join(name)).ok()
}

#[test]
fn issue2635_bare_exec_fd_at_limit_fails() {
    // The issue's seed: `ulimit -n 16; eval "exec 16>/dev/null"`.
    let dir = scratch("bare-at-limit");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16; exec 16>/dev/null; echo rc=$?\n",
    );
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("16: Bad file descriptor"),
        "expected EBADF diagnostic for fd 16, got: {stderr:?}"
    );
}

#[test]
fn issue2635_bare_exec_fd_below_limit_passes() {
    // Control: fd 15 is the highest legal slot under a limit of 16.
    let dir = scratch("bare-below-limit");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16; exec 15>/dev/null; echo rc=$?\n",
    );
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2635_bare_exec_output_at_limit_creates_then_fails() {
    // GNU opens the target (the file exists, empty) and the dup2
    // onto fd 16 is what fails.
    let dir = scratch("bare-output");
    let (stdout, stderr) =
        rubash_script(&dir, "case.sh", "ulimit -n 16; exec 16>f.txt; echo rc=$?\n");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("16: Bad file descriptor"),
        "got: {stderr:?}"
    );
    assert_eq!(file_contents(&dir, "f.txt").as_deref(), Some(""));
}

#[test]
fn issue2635_bare_exec_input_at_limit_fails() {
    let dir = scratch("bare-input");
    fs::write(dir.join("in.txt"), "x\n").expect("write input fixture");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16; exec 16<in.txt; echo rc=$?\n",
    );
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("16: Bad file descriptor"),
        "got: {stderr:?}"
    );
}

#[test]
fn issue2635_bare_exec_dup_at_limit_names_source() {
    // For a dup entry GNU's diagnostic names the duplicated word.
    let dir = scratch("bare-dup");
    let (stdout, stderr) = rubash_script(&dir, "case.sh", "ulimit -n 16; exec 16>&1; echo rc=$?\n");
    assert_eq!(stdout, "rc=1\n");
    assert!(stderr.contains("1: Bad file descriptor"), "got: {stderr:?}");
}

#[test]
fn issue2635_failure_lands_at_its_list_position() {
    // `16>b 9>a`: b is created, the walk stops, a is never created.
    let dir = scratch("position-first");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16; exec 16>b.txt 9>a.txt; echo rc=$?\n",
    );
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("16: Bad file descriptor"),
        "got: {stderr:?}"
    );
    assert_eq!(file_contents(&dir, "b.txt").as_deref(), Some(""));
    assert_eq!(file_contents(&dir, "a.txt"), None);
    // `9>a 16>b`: both targets are created before the failure.
    let dir = scratch("position-second");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16; exec 9>a.txt 16>b.txt; echo rc=$?\n",
    );
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("16: Bad file descriptor"),
        "got: {stderr:?}"
    );
    assert_eq!(file_contents(&dir, "a.txt").as_deref(), Some(""));
    assert_eq!(file_contents(&dir, "b.txt").as_deref(), Some(""));
}

#[test]
fn issue2635_exec_with_command_at_limit_never_runs() {
    let dir = scratch("exec-command");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16; ( exec echo hi 16>f.txt ); echo rc=$?\n",
    );
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("16: Bad file descriptor"),
        "got: {stderr:?}"
    );
    assert_eq!(file_contents(&dir, "f.txt").as_deref(), Some(""));
}

#[test]
fn issue2635_external_command_at_limit_never_runs() {
    let dir = scratch("external");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16; /bin/echo hi 16>f.txt; echo rc=$?\n",
    );
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("16: Bad file descriptor"),
        "got: {stderr:?}"
    );
    assert_eq!(file_contents(&dir, "f.txt").as_deref(), Some(""));
}

#[test]
fn issue2635_builtin_at_limit_never_runs() {
    let dir = scratch("builtin");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16; echo hi 16>f.txt; echo rc=$?\n",
    );
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("16: Bad file descriptor"),
        "got: {stderr:?}"
    );
    assert_eq!(file_contents(&dir, "f.txt").as_deref(), Some(""));
}

#[test]
fn issue2635_compound_at_limit_never_runs() {
    let dir = scratch("compound");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16; { echo hi; } 16>f.txt; echo rc=$?\n",
    );
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("16: Bad file descriptor"),
        "got: {stderr:?}"
    );
    assert_eq!(file_contents(&dir, "f.txt").as_deref(), Some(""));
}

#[test]
fn issue2635_close_at_limit_is_exempt() {
    // Control: r_close_this ignores the close result — closing an
    // over-limit fd is not an error in GNU.
    let dir = scratch("close-exempt");
    let (stdout, stderr) = rubash_script(&dir, "case.sh", "ulimit -n 16; exec 20>&-; echo rc=$?\n");
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2635_numbered_fd_below_limit_still_works() {
    // Control: a legal numbered bind under the limit keeps working.
    let dir = scratch("below-limit-works");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "ulimit -n 16; exec 10>f.txt; echo rc=$?; echo hi >&10\n",
    );
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(file_contents(&dir, "f.txt").as_deref(), Some("hi\n"));
}
