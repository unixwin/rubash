//! rubash#2171 — a child that merely INSPECTS a descriptor
//! through a `/proc/self/fd/N` operand (e.g. `ls -l
//! /proc/self/fd/0`) destroyed the parent's binding: the unix
//! operand materialization recorded the parent FileFd's own handle
//! in the attach record, and the record's Drop — which closes
//! every recorded handle once the child exits, on the contract
//! that the record owns what it attaches — closed the parent's
//! live descriptor. Every later parent read on that fd failed
//! EBADF; for an exec-bound directory the read builtin reported
//! `Bad file descriptor` where GNU reports the descriptor's own
//! `Is a directory` (read.def:911).
//!
//! The repair keeps the record's ownership contract: the File arm
//! attaches a DUPLICATE of the endpoint handle (dup shares the
//! open file description, offset included, so the child's view is
//! unchanged) and the record closes only its dup. The materialized
//! Windows path never hands the child the parent handle (it passes
//! paths/temp files and its consume step seeks, never closes), so
//! the defect and the fix are unix-branch only.
//!
//! Expectations hand-measured against GNU bash 5.3.20 via script
//! files (probes p2171_a..e in the cluster workdir).

use std::path::PathBuf;
use std::process::Command;

fn fixture_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2171-pins-{tag}"));
    let sub = dir.join("d");
    std::fs::create_dir_all(&sub).expect("create fixture dir");
    std::fs::write(dir.join("input.txt"), b"hello-from-input\nsecond-line\n")
        .expect("write fixture");
    dir
}

/// Runs the script with `$D` bound to a fixture directory and `$F`
/// to a regular fixture file inside it.
fn run_with_fixture(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = fixture_dir(tag);
    let shell_d = dir.join("d").to_string_lossy().replace('\\', "/");
    let shell_f = dir.join("input.txt").to_string_lossy().replace('\\', "/");
    let script = script.replace("$D", &shell_d).replace("$F", &shell_f);
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", &script])
        .stdin(std::process::Stdio::null())
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue2171_directory_fd_survives_child_proc_view() {
    let (stdout, stderr, code) = run_with_fixture(
        "dirview",
        "exec 0<$D\n\
         ls -l /proc/self/fd/0 > /dev/null\n\
         if read x; then echo read-ok; else echo read-fail; fi\n",
    );
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("read-fail"),
        "the read still fails: {stdout}"
    );
    assert!(
        stderr.contains("Is a directory"),
        "the descriptor's own EISDIR shape survives: {stderr}"
    );
    assert!(
        !stderr.contains("Bad file descriptor"),
        "the parent descriptor must not be dead: {stderr}"
    );
}

#[test]
fn issue2171_numbered_file_fd_survives_child_proc_view() {
    let (stdout, stderr, code) = run_with_fixture(
        "fd3view",
        "exec 3<$F\n\
         ls -l /proc/self/fd/3 > /dev/null\n\
         if read -u 3 line; then echo \"read-ok [$line]\"; else echo read-fail; fi\n",
    );
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("read-ok [hello-from-input]"),
        "fd 3 still reads after the child's inspection: {stdout}"
    );
    assert!(
        !stderr.contains("Bad file descriptor"),
        "the parent descriptor must not be dead: {stderr}"
    );
}

#[test]
fn issue2171_repeated_proc_views_leave_fd_intact() {
    let (stdout, stderr, code) = run_with_fixture(
        "twoviews",
        "exec 0<$F\n\
         ls -l /proc/self/fd/0 > /dev/null\n\
         ls -l /proc/self/fd/0 > /dev/null\n\
         if read line; then echo \"read-ok [$line]\"; else echo read-fail; fi\n",
    );
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("read-ok [hello-from-input]"),
        "fd 0 still reads after two inspections: {stdout}"
    );
    assert!(
        !stderr.contains("Bad file descriptor"),
        "the parent descriptor must not be dead: {stderr}"
    );
}

#[test]
fn issue2171_directory_read_error_shape_without_any_child() {
    // Guard (green both ways, the #2101/#2167 contract): with no
    // inspecting child the directory read already reports EISDIR.
    let (stdout, stderr, code) = run_with_fixture(
        "dirplain",
        "exec 0<$D\nif read x; then echo read-ok; else echo read-fail; fi\n",
    );
    assert_eq!(code, Some(0));
    assert!(stdout.contains("read-fail"), "the read fails: {stdout}");
    assert!(stderr.contains("Is a directory"), "EISDIR shape: {stderr}");
}
