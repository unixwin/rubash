//! rubash#3655: heredoc vs plain-file redirect on the SAME fd — GNU
//! applies redirects left-to-right and the last one wins (redir.c
//! do_redirection_internal walks the list in order; the r_reading_until
//! case calls here_document_to_fd at redir.c:1065-1072 and then
//! dup2's onto the redirector — each entry replaces the descriptor
//! wholesale). Rubash's fd table kept input/output arms independent
//! (`open_input` replaced only the read arm, `open_output` only the
//! write arm), so after `exec 14<<A 14>file` fd 14 still carried the
//! heredoc's read arm and `read -u 14` saw `first` on a descriptor
//! GNU leaves write-only. The fix makes both primitives retire the
//! opposite arm (dup2 wholesale) and routes every legitimate dual-arm
//! `<>` install through a single `open_readwrite`.
//!
//! Every expectation below is byte-verified against GNU bash 5.3.20
//! (oracle probe ~/workspace/fix-3655/probe-oracle.sh): the failing
//! reads report `rc=1` with `read: 14: read error: Bad file
//! descriptor` on stderr, the write arm that wins still accepts
//! writes, and the `<>` pair keeps both arms on one shared offset.
//! The `ulimit -n 16` preludes mirror the issue3578 pin's harness so
//! fd 14 stays below the soft limit.

use std::path::PathBuf;
use std::process::Command;

fn run_script(name: &str, script: &str) -> (String, String, Option<i32>, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rubash-issue3655-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create probe directory");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .env_remove("BASH_ENV")
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
        dir,
    )
}

fn read_probe_file(dir: &PathBuf, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).unwrap_or_else(|_| panic!("probe file {name} missing"))
}

#[test]
fn issue_heredoc_then_truncate_file_wins() {
    // The issue shape: the truncate bind retires the heredoc's read
    // arm, so fd 14 is write-only — the read fails EBADF exactly like
    // GNU, and the write arm that won still accepts writes.
    let (stdout, stderr, code, dir) = run_script(
        "hd-trunc",
        "ulimit -n 16\n\
         exec 14<<A 14>cross.txt\n\
         first\n\
         A\n\
         read -u 14 x; echo \"rc=$? x=$x\"\n\
         echo hi >&14; echo \"wrc=$?\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "rc=1 x=\nwrc=0\n", "stdout={stdout:?}");
    assert!(
        stderr.contains("read: 14: read error: Bad file descriptor"),
        "stderr={stderr:?}"
    );
    assert_eq!(read_probe_file(&dir, "cross.txt"), "hi\n");
}

#[test]
fn issue_heredoc_then_append_file_wins() {
    // Same last-wins face with `>>`: the append bind retires the
    // heredoc read arm; the file is created, empty, never read.
    let (stdout, stderr, code, dir) = run_script(
        "hd-append",
        "ulimit -n 16\n\
         exec 14<<A 14>>cross.txt\n\
         first\n\
         A\n\
         read -u 14 x; echo \"rc=$? x=$x\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "rc=1 x=\n", "stdout={stdout:?}");
    assert!(
        stderr.contains("read: 14: read error: Bad file descriptor"),
        "stderr={stderr:?}"
    );
    assert_eq!(read_probe_file(&dir, "cross.txt"), "");
}

#[test]
fn issue_truncate_then_heredoc_heredoc_wins() {
    // Reverse-order control: the heredoc entry is last, so fd 14
    // stays bound to the heredoc body and the read succeeds.
    let (stdout, stderr, code, dir) = run_script(
        "trunc-hd",
        "ulimit -n 16\n\
         exec 14>cross.txt 14<<A\n\
         first\n\
         A\n\
         read -u 14 x; echo \"rc=$? x=$x\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "rc=0 x=first\n", "stdout={stdout:?}");
    assert_eq!(stderr, "", "stderr={stderr:?}");
    assert_eq!(read_probe_file(&dir, "cross.txt"), "");
}

#[test]
fn issue_append_then_heredoc_heredoc_wins() {
    // Reverse-order control with `>>`.
    let (stdout, stderr, code, dir) = run_script(
        "append-hd",
        "ulimit -n 16\n\
         exec 14>>cross.txt 14<<A\n\
         first\n\
         A\n\
         read -u 14 x; echo \"rc=$? x=$x\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "rc=0 x=first\n", "stdout={stdout:?}");
    assert_eq!(stderr, "", "stderr={stderr:?}");
    assert_eq!(read_probe_file(&dir, "cross.txt"), "");
}

#[test]
fn issue_heredoc_then_input_file_wins() {
    // Input-vs-input last-wins: a plain input file after the heredoc
    // replaces the read arm with the file's contents.
    let (stdout, stderr, code, _dir) = run_script(
        "hd-input",
        "ulimit -n 16\n\
         printf \"fromfile\\n\" > input.txt\n\
         exec 14<<A 14<input.txt\n\
         first\n\
         A\n\
         read -u 14 x; echo \"rc=$? x=$x\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "rc=0 x=fromfile\n", "stdout={stdout:?}");
    assert_eq!(stderr, "", "stderr={stderr:?}");
}

#[test]
fn issue_input_file_then_heredoc_heredoc_wins() {
    // Reverse-order control: the heredoc after the input file wins.
    let (stdout, stderr, code, _dir) = run_script(
        "input-hd",
        "ulimit -n 16\n\
         printf \"fromfile\\n\" > input.txt\n\
         exec 14<input.txt 14<<A\n\
         first\n\
         A\n\
         read -u 14 x; echo \"rc=$? x=$x\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "rc=0 x=first\n", "stdout={stdout:?}");
    assert_eq!(stderr, "", "stderr={stderr:?}");
}

#[test]
fn issue_readwrite_pair_keeps_both_arms() {
    // The `<>` regression guard for the open_readwrite conversion:
    // ONE description serves both directions on a shared offset —
    // the read consumes `first`, the write lands right after it.
    let (stdout, stderr, code, dir) = run_script(
        "rw-pair",
        "ulimit -n 16\n\
         printf \"first\\n\" > rw.txt\n\
         exec 14<>rw.txt\n\
         read -u 14 x; echo \"rc=$? x=$x\"\n\
         echo second >&14; echo \"wrc=$?\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "rc=0 x=first\nwrc=0\n", "stdout={stdout:?}");
    assert_eq!(stderr, "", "stderr={stderr:?}");
    assert_eq!(read_probe_file(&dir, "rw.txt"), "first\nsecond\n");
}

#[test]
fn issue_here_string_then_truncate_file_wins() {
    // The here-string face of the issue: `<<<` installs a read arm
    // the same way, so the later truncate retires it too.
    let (stdout, stderr, code, dir) = run_script(
        "hs-trunc",
        "ulimit -n 16\n\
         exec 14<<<word 14>cross.txt\n\
         read -u 14 x; echo \"rc=$? x=$x\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "rc=1 x=\n", "stdout={stdout:?}");
    assert!(
        stderr.contains("read: 14: read error: Bad file descriptor"),
        "stderr={stderr:?}"
    );
    assert_eq!(read_probe_file(&dir, "cross.txt"), "");
}

#[test]
fn issue_truncate_then_here_string_here_string_wins() {
    // Reverse-order control for the here-string face.
    let (stdout, stderr, code, dir) = run_script(
        "trunc-hs",
        "ulimit -n 16\n\
         exec 14>cross.txt 14<<<word\n\
         read -u 14 x; echo \"rc=$? x=$x\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "rc=0 x=word\n", "stdout={stdout:?}");
    assert_eq!(stderr, "", "stderr={stderr:?}");
    assert_eq!(read_probe_file(&dir, "cross.txt"), "");
}

#[test]
fn issue_fd0_heredoc_then_truncate_file_wins() {
    // The fd-0 face: `exec 0<<A 0>out.txt` leaves fd 0 write-only,
    // so a plain `read` fails EBADF exactly like GNU.
    let (stdout, stderr, code, dir) = run_script(
        "fd0-hd-trunc",
        "exec 0<<A 0>out.txt\n\
         first\n\
         A\n\
         read x; echo \"rc=$? x=$x\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "rc=1 x=\n", "stdout={stdout:?}");
    // The ordering face is the pin: the heredoc read arm is retired,
    // so `read` fails with an empty x instead of seeing "first".
    // The missing EBADF diagnostic on this fd-0 shape is a separate
    // read-builtin face — rubash#3770 (GNU prints
    // `read: 0: read error: Bad file descriptor`; rubash prints
    // nothing here) — deliberately NOT asserted until #3770 lands.
    let _ = stderr;
    assert_eq!(read_probe_file(&dir, "out.txt"), "");
}
