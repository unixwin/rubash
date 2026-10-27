//! rubash#1624 — after a coprocess child died, its NAME array and
//! NAME_PID stayed bound forever (`fds=[63 -1] pid=[<dead pid>]`)
//! whenever both pipe ends had already left the fd table before the
//! reap: the write end closed by `exec {NAME[1]}>&-` and the read end
//! closed by the EOF path of an earlier `read -t`.
//!
//! Two mechanism fixes:
//! - read_coproc_stdout conflated "the read loop ended" with "the
//!   pipe hit EOF": a deadline break (or a SIGINT abort) also closed
//!   the shell-owned read endpoint. GNU keeps the descriptor across
//!   builtin calls (read.def) — only a true EOF (empty read or read
//!   error) may close it, so a timed-out read leaves the coproc fully
//!   usable.
//! - retire_completed_coproc recognized a reaped pid as a coproc
//!   only by scanning live fd_table endpoints. GNU coproc_reap
//!   (execute_cmd.c) walks the coproc list and runs
//!   coproc_unsetvars (execute_cmd.c:2450) for the pid; rubash's
//!   registry is coproc_names, so recognition is keyed on it (the
//!   endpoint scan remains as a fallback).
//!
//! Expectations verified against GNU bash 5.3.20 script-file probes.

use std::process::{Command, Stdio};

fn rubash_script(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-1624-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir(&dir).expect("create scratch dir");
    let path = dir.join("s.sh");
    std::fs::write(&path, script).expect("seed script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let result = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn coproc_family_retires_after_read_end_eof() {
    // The issue's regression seed verbatim (with plain `sleep`):
    // the write end closes first, the child dies, the read hits true
    // EOF — which closes the read endpoint — and the reap must still
    // retire the whole family.
    let (stdout, stderr, code) = rubash_script(
        "coproc MYC { cat; }\n\
         exec {MYC[1]}>&-\n\
         read -u ${MYC[0]} line; echo \"eof-read rc=$?\"\n\
         sleep 1\n\
         echo \"settled: fds=[${MYC[*]}] pid=[${MYC_PID-}]\"\n\
         read -u ${MYC[0]} again; echo \"again rc=$?\"\n",
    );
    assert_eq!(
        stdout, "eof-read rc=1\nsettled: fds=[] pid=[]\nagain rc=1\n",
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("read: again: invalid file descriptor specification"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn coproc_family_retires_after_timed_read_and_stdin_close() {
    // Companion ordering: a successful read first, then the write end
    // closes and the child dies — the family must still be unbound.
    let (stdout, stderr, code) = rubash_script(
        "coproc MYC { cat; }\n\
         echo hello >&${MYC[1]}\n\
         read -u ${MYC[0]} line; echo \"read1 rc=$? line=[$line]\"\n\
         exec {MYC[1]}>&-\n\
         sleep 0.3\n\
         echo \"fds=[${MYC[@]}] pid=[${MYC_PID}]\"\n",
    );
    assert_eq!(
        stdout, "read1 rc=0 line=[hello]\nfds=[] pid=[]\n",
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn coproc_timed_out_read_leaves_descriptor_usable() {
    // GNU: a `read -t` timeout (rc 142) does not close the coproc
    // descriptor — the next write/read round-trips normally, and the
    // family still retires once the child dies.
    let (stdout, stderr, code) = rubash_script(
        "coproc MYC { cat; }\n\
         read -t 0.1 -u ${MYC[0]} l1; echo \"r1 rc=$? l1=[$l1]\"\n\
         echo hello >&${MYC[1]}\n\
         read -u ${MYC[0]} l2; echo \"r2 rc=$? l2=[$l2]\"\n\
         exec {MYC[0]}>&- {MYC[1]}>&-\n\
         sleep 0.2\n\
         echo \"final: fds=[${MYC[@]}] pid=[${MYC_PID}]\"\n",
    );
    assert_eq!(
        stdout, "r1 rc=142 l1=[]\nr2 rc=0 l2=[hello]\nfinal: fds=[] pid=[]\n",
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn coproc_family_retires_and_reread_names_the_variable() {
    // After retirement the array is gone, so a re-read fails on the
    // empty fd operand exactly like GNU (`invalid file descriptor
    // specification`), not on a stale descriptor number.
    let (stdout, stderr, code) = rubash_script(
        "coproc MYC { cat; }\n\
         exec {MYC[1]}>&-\n\
         sleep 0.3\n\
         echo \"fds=[${MYC[@]}] pid=[${MYC_PID}]\"\n\
         read -u ${MYC[0]} again; echo \"again rc=$?\"\n",
    );
    assert_eq!(stdout, "fds=[] pid=[]\nagain rc=1\n", "stderr: {stderr}");
    assert!(
        stderr.contains("read: again: invalid file descriptor specification"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}
