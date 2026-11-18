//! rubash#3475 rework regressions: an async coproc child writing to a
//! dead coproc-output pipe must die silently (SIGPIPE), not emit a
//! shell `Broken pipe` diagnostic on stderr.
//!
//! Disclosed as Windows platform debt during the #3475 redirect-first
//! coproc rework: `coproc <<HD cat` intermittently printed
//! `case.sh: line 1: Broken pipe` when the parent exited (or retired
//! the coproc read end via `exec {COPROC[0]}<&-`) before the child's
//! output write landed. Root cause was not the pipe lifetime — GNU
//! models the coproc body as a forked child, and forked children run
//! with SIGPIPE at SIG_DFL (execute_cmd.c:1670 → trap.c:1480), so a
//! write to a readerless pipe kills the child quietly with status 141.
//! rubash respawns coproc bodies as `rubash -c` children on Windows
//! and routed the child's EPIPE through the main-shell
//! `report_io_error_routed` diagnostic path, printing `Broken pipe`.
//! Fix: a validated respawn child converts EPIPE to
//! `ExecuteError::BrokenPipe` (silent, exit 141); the main shell keeps
//! the #440 report-and-continue model. The cells below pin the
//! respawn-child verdict, deterministic by having the child `sleep`
//! past the parent's close of the read end.
//!
//! Expectations verified byte-for-byte against GNU Bash 5.3.20
//! script-file probes (WSL /usr/local/bin/bash oracle, LF scripts):
//! GNU prints no diagnostic, `wait` reports status 0 (the reaped job
//! is gone from the list), the script continues, rc 0.

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue3475-bp-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn check(script: &str, expected_stdout: &str, expected_stderr: &str, expected_code: i32) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(
        code,
        Some(expected_code),
        "script: {script:?}\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert_eq!(stdout, expected_stdout, "script: {script:?}");
    assert_eq!(stderr, expected_stderr, "script: {script:?}");
}

/// Builtin write in a brace-group coproc child after the read end is
/// gone: the child's `echo` hits EPIPE and the child dies silently.
/// GNU 5.3.20: stderr empty, `wait` -> 0, rc 0.
#[test]
fn issue3475_coproc_child_builtin_write_dead_pipe_is_silent() {
    check(
        "coproc { sleep 0.5; echo hi; }\n\
         exec {COPROC[0]}<&-\n\
         wait\n\
         echo status=$?\n\
         echo after\n",
        "status=0\nafter\n",
        "",
        0,
    );
}

/// Emulated-external `cat` write to a dead coproc pipe: same silent
/// SIGPIPE verdict inside the respawned child — this is the concrete
/// path that produced the disclosed `Broken pipe` diagnostic (the
/// child's EPIPE bubbled to `report_io_error_routed`).
#[test]
fn issue3475_coproc_child_emulated_external_dead_pipe_is_silent() {
    check(
        "seq 1 200000 > big\n\
         coproc { sleep 0.5; cat big; }\n\
         exec {COPROC[0]}<&-\n\
         wait\n\
         echo status=$?\n\
         echo after\n",
        "status=0\nafter\n",
        "",
        0,
    );
}

/// The disclosed #3475 shape, made deterministic: heredoc-fed coproc
/// child sleeps past the parent's close of the read end, then `cat`
/// echoes the heredoc body into a dead pipe — silent, `done`, rc 0.
#[test]
fn issue3475_coproc_heredoc_body_dead_pipe_is_silent() {
    check(
        "coproc { sleep 0.5; cat; } <<HD\n\
         hello\n\
         HD\n\
         exec {COPROC[0]}<&-\n\
         wait\n\
         echo done\n",
        "done\n",
        "",
        0,
    );
}

/// A brace-group coproc child continues past the dead write (the
/// write dies with EPIPE inside the group; later commands still run),
/// matching GNU: `inner.txt` gets `after-inner`, parent stays clean.
#[test]
fn issue3475_coproc_child_group_continues_after_dead_write() {
    check(
        "coproc { sleep 0.5; cat case.sh; echo after-inner >inner.txt; }\n\
         exec {COPROC[0]}<&-\n\
         wait\n\
         echo status=$?\n\
         cat inner.txt\n",
        "status=0\nafter-inner\n",
        "",
        0,
    );
}
