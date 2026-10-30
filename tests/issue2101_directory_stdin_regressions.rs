//! rubash#2101 — a directory as a command's stdin was killed at
//! OPEN time: `open_classified_input`'s SyntheticLink arm (and the
//! fd layer's plain open) refused the directory, so the command
//! never ran. GNU opens the directory fine (redir.c redir_open),
//! hands the descriptor to the child, and only a READING child
//! fails — with its own diagnostic (`cat: -: Is a directory`);
//! a non-reading command (`ls < dir`) runs normally, status 0.
//!
//! Fix shape: (1) the Windows fd layer retries a directory open
//! with FILE_FLAG_BACKUP_SEMANTICS (open_file_read), and the
//! executor's std-File input opens route through the same retry —
//! on Windows the filed defect was the open itself; (2) a
//! compound's fd-0 stream class admits directories, so the bind is
//! a real descriptor instead of the text mirror that swallowed the
//! read error at bind time; (3) the text stdin ladder declines a
//! hard read error instead of serving it as an empty stream, and
//! FdTable::read_all_bytes records the first hard error so the
//! emulated cat reports `cat: -: Is a directory` with status 1,
//! exactly like a real child (read.def:911 owns the read builtin's
//! own diagnostic shape).
//!
//! Expectations hand-measured against GNU bash 5.3.20 via script
//! files. On Linux several faces were already correct for simple
//! commands (the Unix open never refused); the compound / exec /
//! drain faces below are the ones that were red on master there.
//! The Windows open faces are adjudicated by the Windows CI job —
//! the pins state the GNU contract uniformly.

use std::path::PathBuf;
use std::process::Command;

fn fixture_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2101-pins-{tag}"));
    let sub = dir.join("d");
    std::fs::create_dir_all(&sub).expect("create fixture dir");
    std::fs::write(dir.join("input.txt"), b"hello world\n").expect("write fixture");
    dir
}

/// Runs the script with `$D` bound to the fixture directory and
/// `$F` to a regular fixture file inside it.
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
fn issue2101_reading_external_gets_the_descriptor_and_fails_itself() {
    let (stdout, stderr, code) =
        run_with_fixture("simple", "cat < $D\necho rc=$?\ncat < $D/\necho rc2=$?\n");
    assert_eq!(code, Some(0));
    assert!(stdout.contains("rc=1"), "cat must fail itself: {stdout}");
    assert!(stdout.contains("rc2=1"), "trailing-slash form: {stdout}");
    assert!(
        stderr.contains("cat: -: Is a directory"),
        "GNU diagnostic shape: {stderr}"
    );
}

#[test]
fn issue2101_non_reading_external_runs_fine() {
    let (stdout, _stderr, code) = run_with_fixture(
        "nonreader",
        "ls < $D > /dev/null\necho rc=$?\nls < /proc/self/cwd > /dev/null\necho rc2=$?\n",
    );
    assert_eq!(code, Some(0));
    assert!(stdout.contains("rc=0"), "ls must run: {stdout}");
    assert!(stdout.contains("rc2=0"), "synthetic-link dir: {stdout}");
}

#[test]
fn issue2101_synthetic_link_directory_reaches_the_reader() {
    let (stdout, stderr, code) = run_with_fixture("synlink", "cat < /proc/self/cwd\necho rc=$?\n");
    assert_eq!(code, Some(0));
    assert!(stdout.contains("rc=1"), "reader must fail: {stdout}");
    assert!(
        stderr.contains("cat: -: Is a directory"),
        "GNU diagnostic shape: {stderr}"
    );
}

#[test]
fn issue2101_compound_binds_the_descriptor_not_the_mirror() {
    let (stdout, stderr, code) = run_with_fixture(
        "compound",
        "{ cat; } < $D\necho cat_rc=$?\n\
         { [ -d /dev/stdin ]; } < $D && echo fact=T || echo fact=F\n",
    );
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("cat_rc=1"),
        "compound cat must fail: {stdout}"
    );
    assert!(
        stderr.contains("cat: -: Is a directory"),
        "GNU diagnostic shape: {stderr}"
    );
    assert!(
        stdout.contains("fact=T"),
        "the bound descriptor is the directory: {stdout}"
    );
}

#[test]
fn issue2101_compound_read_reports_the_read_error() {
    let (stdout, stderr, code) = run_with_fixture(
        "grp_read",
        "{ read x; } < $D\necho rc=$?\n\
         while read x; do echo body; done < $D\necho loop_rc=$?\n",
    );
    assert_eq!(code, Some(0));
    assert!(stdout.contains("rc=1"), "read must fail: {stdout}");
    assert!(
        stdout.contains("loop_rc=0"),
        "a failed read ends the loop with status 0: {stdout}"
    );
    assert!(!stdout.contains("body"), "loop body never runs: {stdout}");
    assert!(
        stderr.contains("Is a directory"),
        "the read diagnostic is printed, not swallowed: {stderr}"
    );
}

#[test]
fn issue2101_exec_bound_directory_drains_fail_like_gnu() {
    let (stdout, stderr, code) = run_with_fixture(
        "exec",
        "exec 0<$D\ncat\necho cat_rc=$?\ncat -\necho dash_rc=$?\n",
    );
    assert_eq!(code, Some(0));
    assert!(stdout.contains("cat_rc=1"), "cat must fail: {stdout}");
    assert!(stdout.contains("dash_rc=1"), "cat - must fail: {stdout}");
    assert!(
        stderr.contains("cat: -: Is a directory"),
        "GNU diagnostic shape: {stderr}"
    );
}
