//! rubash#575: `exec cmd` inside a command substitution must keep the
//! inherited stdin channel.
//!
//! GNU ground truth (WSL bash 5.3.0, byte-compared matrix): subst.c
//! command_substitute dups ONLY fd 1 onto the capture pipe
//! (subst.c:7143-7340) and leaves fd 0 inherited, and exec.def
//! exec_builtin -> shell_execve replaces the shell image in place — so
//! `$(exec cat)` reads the outer stdin in every context (pipe, file,
//! read-then-exec, herestring, `<&N` dup). rubash's exec builtin never set
//! a child stdin on any spawn path and std's `Command::output()` defaults
//! stdin to `Stdio::null()`, so the exec'd program read EOF: modernish's
//! stdin-script mode (`echo script | bin/modernish`, whose tail is
//! `eval "... $(exec cat)"`) ran only its first line.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_stdin(args: &[&str], stdin: &[u8]) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(stdin)
        .expect("write stdin");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The minimal repro: `$(exec cat)` in a `-c` program reads the piped
/// stdin to EOF (GNU: `r=[l1\nl2]`; rubash#575 read `r=[]`).
#[test]
fn comsub_exec_cat_reads_pipe_stdin() {
    let (stdout, stderr, rc) = run_stdin(&["-c", "r=$(exec cat); echo \"r=[$r]\""], b"l1\nl2\n");
    assert_eq!(stdout, "r=[l1\nl2]\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}

/// modernish stdin-script shape: `read` consumes the hashbang line, then
/// `eval "$(exec cat)"` must receive the rest (GNU: `[l1|l2\nl3]`;
/// rubash#575 dropped everything after the first read).
#[test]
fn read_then_comsub_exec_cat_keeps_remainder() {
    let (stdout, stderr, rc) = run_stdin(
        &["-c", "read -r x; r=$(exec cat); echo \"[$x|$r]\""],
        b"l1\nl2\nl3\n",
    );
    assert_eq!(stdout, "[l1|l2\nl3]\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}

/// A command substitution whose body is the first consumer still reads the
/// whole stream (`printf 'p\nq\n' | { printf 'noread=[%s]\n' "$(exec
/// cat)"; }`, GNU: `noread=[p\nq]`).
#[test]
fn comsub_noread_exec_cat_reads_whole_stream() {
    let (stdout, stderr, rc) = run_stdin(
        &["-c", "printf 'noread=[%s]\\n' \"$(exec cat)\""],
        b"p\nq\n",
    );
    assert_eq!(stdout, "noread=[p\nq]\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}

/// Top-level command substitution around exec, no surrounding read
/// (GNU: `[ab]`).
#[test]
fn top_level_comsub_exec_cat() {
    let (stdout, stderr, rc) = run_stdin(&["-c", "r=$(exec cat); echo \"[$r]\""], b"ab\n");
    assert_eq!(stdout, "[ab]\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}

/// `exec cat` as a pipeline element reads the pipe, not EOF (GNU:
/// `[pl]`; rubash#575's observed matrix row).
#[test]
fn pipeline_exec_cat_feeds_downstream() {
    let (stdout, stderr, rc) = run_stdin(
        &[
            "-c",
            "printf 'pl\\n' | { exec cat; } | head -1 | { r=$(cat); echo \"[$r]\"; }",
        ],
        b"unused\n",
    );
    assert_eq!(stdout, "[pl]\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}

/// `$(exec cat <&0)` — an explicit dup of fd 0 is still the inherited
/// channel (GNU: `[ee]`).
#[test]
fn comsub_exec_cat_dup_fd0_reads_stdin() {
    let (stdout, stderr, rc) = run_stdin(&["-c", "r=$(exec cat <&0); echo \"[$r]\""], b"ee\n");
    assert_eq!(stdout, "[ee]\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}

/// A herestring on the exec'd node binds the child's fd 0 to its text
/// (GNU redir.c binds it before execve; GNU: `[hs1]`).
#[test]
fn comsub_exec_cat_herestring() {
    let (stdout, stderr, rc) =
        run_stdin(&["-c", "r=$(exec cat <<< hs1); echo \"[$r]\""], b"unused\n");
    assert_eq!(stdout, "[hs1]\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}

/// `exec cat < file` inside a substitution reads the file, not the shell's
/// own stdin (GNU: `[R1\nR2]`; rubash#575 read the inherited stdin or
/// nothing).
#[test]
fn comsub_exec_cat_file_redirect() {
    let mut path = std::env::temp_dir();
    path.push(format!("rubash-575-{}-in.txt", std::process::id()));
    std::fs::write(&path, b"R1\nR2\n").expect("write fixture");
    let script = format!(
        "r=$(exec cat < '{}'); echo \"[$r]\"",
        path.to_string_lossy().replace('\\', "/")
    );
    let (stdout, stderr, rc) = run_stdin(&["-c", &script], b"unused\n");
    let _ = std::fs::remove_file(&path);
    assert_eq!(stdout, "[R1\nR2]\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}

/// `exec 9<file; $(exec cat <&9)` — a dup of a shell-owned descriptor
/// reaches the exec'd image (GNU dup2 semantics; GNU: `[R1\nR2]`).
#[test]
fn comsub_exec_cat_dup_of_numbered_fd() {
    let mut path = std::env::temp_dir();
    path.push(format!("rubash-575-{}-dup.txt", std::process::id()));
    std::fs::write(&path, b"R1\nR2\n").expect("write fixture");
    let script = format!(
        "exec 9< '{p}'; r=$(exec cat <&9); echo \"[$r]\"; exec 9<&-",
        p = path.to_string_lossy().replace('\\', "/")
    );
    let (stdout, stderr, rc) = run_stdin(&["-c", &script], b"");
    let _ = std::fs::remove_file(&path);
    assert_eq!(stdout, "[R1\nR2]\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}

/// Guards — shapes that already worked on master must not regress:
/// plain `exec cat` at top level inherits (spawn-with-status semantics).
#[test]
fn plain_c_exec_cat_still_inherits() {
    let (stdout, stderr, rc) = run_stdin(&["-c", "exec cat"], b"E1\nE2\n");
    assert_eq!(stdout, "E1\nE2\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}

/// Guards — plain `$(cat)` (no exec) was already correct on master.
#[test]
fn plain_comsub_cat_unaffected() {
    let (stdout, stderr, rc) = run_stdin(&["-c", "r=$(cat); echo \"[$r]\""], b"k1\nk2\n");
    assert_eq!(stdout, "[k1\nk2]\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}

/// Guards — a plain external in a comsub was already correct; its stdin
/// must stay exactly the inherited channel, never a surprise handle.
#[test]
fn plain_comsub_external_unaffected() {
    let (stdout, stderr, rc) = run_stdin(&["-c", "r=$(cat); echo \"[$r]\""], b"x1\nx2\n");
    assert_eq!(stdout, "[x1\nx2]\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}
