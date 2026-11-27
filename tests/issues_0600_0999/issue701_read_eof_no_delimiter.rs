//! niubash#701 R3.2: `read` must fail (rc 1) at EOF without a trailing
//! delimiter when a seekable file is bound to fd 0 by a compound command or
//! an inherited descriptor.
//!
//! GNU read.def sets eof and returns EXECUTION_FAILURE when the input is
//! exhausted before the delimiter (read.def:762-768, :949). The seekable-file
//! fast path (`fd_table::read_file_bytes_buffered`, read.def:737 zreadc +
//! :940 zsyncfd) served the trailing partial record but never set the
//! fd-table eof flag, so `{ read l; } < f` and `read l <&9` reported rc=0 at
//! EOF and the classic `while read` loop ran one extra iteration (rubash
//! 1.4.0 / e785c3c8f610). The simple-command `< f` form is answered on the
//! read_shell_input_file path, which already set the flag — hence it agreed
//! with GNU and is kept as a guard below. Reference semantics verified
//! against WSL GNU bash 5.3.0.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn run(args: &[&str], cwd: &Path) -> (String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = cmd
        .spawn()
        .expect("spawn rubash")
        .wait_with_output()
        .expect("wait rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.code(),
    )
}

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue701-{tag}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// The compound-redirection shape from the issue: `printf 'abc' | { IFS=
/// read -r l; echo "rc=$?"; } < f` (GNU: rc=1).
#[test]
fn compound_redirection_eof_without_newline_fails() {
    let dir = scratch_dir("compound");
    std::fs::write(dir.join("f"), b"abc").expect("write f");
    let (stdout, rc) = run(
        &["-c", "{ IFS= read -r l; echo \"rc=$? l=[$l]\"; } < f"],
        &dir,
    );
    assert_eq!(stdout, "rc=1 l=[abc]\n");
    assert_eq!(rc, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The inherited-descriptor shape from the issue: `exec 9< f; IFS= read -r l
/// <&9; echo $?` (GNU: rc=1).
#[test]
fn inherited_fd_eof_without_newline_fails() {
    let dir = scratch_dir("fd");
    std::fs::write(dir.join("f"), b"abc").expect("write f");
    let (stdout, rc) = run(
        &["-c", "exec 9< f; IFS= read -r l <&9; echo \"rc=$? l=[$l]\""],
        &dir,
    );
    assert_eq!(stdout, "rc=1 l=[abc]\n");
    assert_eq!(rc, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The loop idiom the issue is about: a two-line file whose last line lacks
/// `\n` iterates once, not twice (GNU: n=1).
#[test]
fn while_read_without_trailing_newline_iterates_once() {
    let dir = scratch_dir("while2");
    std::fs::write(dir.join("g"), b"aaa\nbbb").expect("write g");
    let (stdout, rc) = run(
        &[
            "-c",
            "n=0; while IFS= read -r l; do n=$((n+1)); done < g; echo \"n=$n\"",
        ],
        &dir,
    );
    assert_eq!(stdout, "n=1\n");
    assert_eq!(rc, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A single unterminated record is not an iteration at all (GNU: n=0).
#[test]
fn while_read_single_unterminated_record_no_iteration() {
    let dir = scratch_dir("while1");
    std::fs::write(dir.join("g"), b"abc").expect("write g");
    let (stdout, rc) = run(
        &[
            "-c",
            "n=0; while IFS= read -r l; do n=$((n+1)); done < g; echo \"n=$n\"",
        ],
        &dir,
    );
    assert_eq!(stdout, "n=0\n");
    assert_eq!(rc, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Guard: a newline-terminated file still succeeds (rc=0) through the same
/// compound path — the eof flag must track the delimiter, not fire blindly.
#[test]
fn compound_redirection_newline_terminated_succeeds() {
    let dir = scratch_dir("compound-nl");
    std::fs::write(dir.join("f"), b"abc\n").expect("write f");
    let (stdout, rc) = run(
        &["-c", "{ IFS= read -r l; echo \"rc=$? l=[$l]\"; } < f"],
        &dir,
    );
    assert_eq!(stdout, "rc=0 l=[abc]\n");
    assert_eq!(rc, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Guard: the simple-command form already agreed with GNU; it must keep
/// failing at EOF-without-newline.
#[test]
fn simple_redirection_eof_without_newline_fails() {
    let dir = scratch_dir("simple");
    std::fs::write(dir.join("f"), b"abc").expect("write f");
    let (stdout, rc) = run(&["-c", "IFS= read -r l < f; echo \"rc=$? l=[$l]\""], &dir);
    assert_eq!(stdout, "rc=1 l=[abc]\n");
    assert_eq!(rc, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}
