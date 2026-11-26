//! Issue rubash#1626: external commands could not see file descriptors
//! opened persistently with `exec` — every reference in a foreign child
//! failed EBADF and the descriptors were missing from the child's fd
//! table. GNU fork inherits the shell's open descriptors at their
//! numbers, so `exec 9>f; /usr/bin/bash -c 'echo x >&9'` writes into f.
//! Rubash's fd table is virtual: the ambient handles reached only
//! rubash children (the __RUBASH_FD_* env channel restored by init.rs),
//! and the handles are opened O_CLOEXEC, so a foreign child got neither
//! the descriptor nor a usable number. The spawn path now pins each
//! ambient fd-table descriptor onto its slot number in the child
//! (Unix pre_exec dup2), mirroring the fork inheritance.
//!
//! Every test drives a REAL foreign child (/usr/bin/bash) so the env
//! channel cannot mask the result, and asserts the GNU bash 5.3.20
//! behavior measured with the same scripts.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1626-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn rubash_c(script: &str, dir: &Path) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn file_contents(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).unwrap_or_default()
}

#[test]
fn issue1626_exec_fd_write_visible_to_external_child() {
    let dir = tempdir("write");
    let run = rubash_c(
        "exec 9>/dev/null; /usr/bin/bash -c 'echo x >&9'; echo inner_rc=$?",
        &dir,
    );
    assert!(run.stdout.contains("inner_rc=0"), "stdout: {}", run.stdout);
    assert!(
        !run.stderr.contains("Bad file descriptor"),
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn issue1626_exec_fd_read_visible_to_external_child() {
    let dir = tempdir("read");
    std::fs::write(dir.join("data_in.txt"), "DATA\n").expect("seed input");
    let run = rubash_c("exec 8<data_in.txt; /usr/bin/bash -c 'cat <&8'", &dir);
    assert_eq!(run.stdout, "DATA\n", "stderr: {}", run.stderr);
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue1626_exec_fd_readwrite_visible_to_external_child() {
    let dir = tempdir("rw");
    std::fs::write(dir.join("rw.txt"), "R\n").expect("seed rw file");
    let run = rubash_c("exec 9<>rw.txt; /usr/bin/bash -c 'cat <&9'", &dir);
    assert_eq!(run.stdout, "R\n", "stderr: {}", run.stderr);
    let run = rubash_c("exec 9<>rw.txt; /usr/bin/bash -c 'echo W >&9'", &dir);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(file_contents(&dir, "rw.txt"), "W\n");
}

#[test]
fn issue1626_exec_fd_listed_in_child_fd_table() {
    let dir = tempdir("listing");
    let run = rubash_c("exec 9>/dev/null; /usr/bin/bash -c 'ls /proc/$$/fd'", &dir);
    assert!(
        run.stdout.lines().any(|fd| fd.trim() == "9"),
        "child fd listing lacks 9: {}",
        run.stdout
    );
}

#[test]
fn issue1626_exec_fd_listed_in_pipeline_child_known_gap() {
    let dir = tempdir("listing-pipe");
    let run = rubash_c(
        "exec 9>/dev/null; /usr/bin/bash -c 'ls /proc/$$/fd' | sort -n | tr '\\n' ' '; echo",
        &dir,
    );
    assert!(
        run.stdout.split_whitespace().any(|fd| fd == "9"),
        "pipeline child fd listing lacks 9: {}",
        run.stdout
    );
}

#[test]
fn issue1688_exec_fd_listed_in_second_pipeline_stage() {
    // rubash#1688: every pipeline member inherits, not just the first.
    let dir = tempdir("listing-pipe2");
    let run = rubash_c(
        "exec 9>/dev/null; echo x | /usr/bin/bash -c 'ls /proc/$$/fd' | sort -n | tr '\\n' ' '; echo",
        &dir,
    );
    assert!(
        run.stdout.split_whitespace().any(|fd| fd == "9"),
        "second-stage fd listing lacks 9: {}",
        run.stdout
    );
}

#[test]
fn issue1688_pipeline_member_writes_through_ambient_fd() {
    // rubash#1688: a write through the ambient descriptor from inside a
    // pipeline member lands in the bound file (GNU fork inheritance).
    let dir = tempdir("pipewrite");
    let run = rubash_c(
        "exec 8>pw.txt; /usr/bin/bash -c 'echo via_pipe >&8' | cat; echo \"pw=[$(cat pw.txt)]\"",
        &dir,
    );
    assert_eq!(run.stdout, "pw=[via_pipe]\n", "stderr: {}", run.stderr);
}

#[test]
fn issue1626_exec_fd_visible_inside_command_substitution() {
    let dir = tempdir("comsub");
    let run = rubash_c(
        "exec 9>cs.txt; x=$(/usr/bin/bash -c 'echo sub >&9; echo done'); echo \"x=[$x] cs=[$(cat cs.txt)]\"",
        &dir,
    );
    assert_eq!(run.stdout, "x=[done] cs=[sub]\n", "stderr: {}", run.stderr);
}

#[test]
fn issue1626_exec_fd_visible_inside_process_substitution() {
    let dir = tempdir("procsub");
    let run = rubash_c(
        "exec 9>ps.txt; cat <(/usr/bin/bash -c 'echo p >&9; echo viaps'); echo \"ps=[$(cat ps.txt)]\"",
        &dir,
    );
    assert_eq!(run.stdout, "viaps\nps=[p]\n", "stderr: {}", run.stderr);
}

#[test]
fn issue1626_exec_fd_visible_inside_nested_command_substitution() {
    let dir = tempdir("nested");
    let run = rubash_c(
        "exec 9>nest.txt; x=$(echo \"$(/usr/bin/bash -c 'echo deep >&9; echo inner')\"); echo \"x=[$x] nest=[$(cat nest.txt)]\"",
        &dir,
    );
    assert_eq!(
        run.stdout, "x=[inner] nest=[deep]\n",
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn issue1626_two_exec_fds_visible_to_external_child() {
    let dir = tempdir("two");
    let run = rubash_c(
        "exec 9>o9.txt 7>o7.txt; /usr/bin/bash -c 'echo nine >&9; echo seven >&7'; echo \"o9=[$(cat o9.txt)] o7=[$(cat o7.txt)]\"",
        &dir,
    );
    assert_eq!(
        run.stdout, "o9=[nine] o7=[seven]\n",
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn issue1626_command_redirect_never_writes_ambient_file() {
    // Safety invariant of the pin: when the command itself redirects a
    // slot, the ambient pin for that slot is skipped, so the child can
    // never be handed the AMBIENT file under the command's redirect —
    // GNU gives the child the command's own binding (own.txt). Delivery
    // of command-level fd redirects to foreign children is a separate
    // pre-existing gap (the child still gets EBADF there today); what
    // this fix must guarantee is that it never makes it worse by
    // writing into the wrong file.
    let dir = tempdir("override");
    let run = rubash_c(
        "exec 9>ambient.txt; /usr/bin/bash -c 'echo own >&9' 9>own.txt; echo \"ambient=[$(cat ambient.txt)]\"",
        &dir,
    );
    assert_eq!(file_contents(&dir, "ambient.txt"), "");
    assert!(run.stdout.contains("ambient=[]"), "stdout: {}", run.stdout);
}
