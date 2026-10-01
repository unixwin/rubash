//! rubash#368: pre-opened fds 3/4 must stay visible inside `$()` command
//! substitutions — `{ v=$(cmd 3>&1 1>&4); } 4>&1` prints the fd-1 half
//! outside and captures the fd-3 half (the nvm-exec `nvm_rc_version`
//! protocol).
//!
//! GNU contract (C source is the specification):
//! - subst.c:7143 `command_substitute` forks the child; at subst.c:7320 it
//!   installs the capture pipe with `dup2 (fildes[1], 1)` — ONLY fd 1 is
//!   replaced, and it is a NEW open file description inheriting none of the
//!   parent fd 1's dup2 aliases. Every other descriptor (fd 3/4 bound by an
//!   enclosing group redirect or `exec N>&M`) survives the fork verbatim.
//! - redir.c:1170 `do_redirection_internal`: `N>&M` is
//!   `dup2 (redir_fd, redirector)` against that inherited table, so inside
//!   the body `3>&1` binds fd 3 to the CAPTURE pipe while `1>&4` moves the
//!   child's stdout to the outer fd 4 — the juggle that routes progress
//!   outside and value into the substitution.
//!
//! Root cause on the rubash side: a dup-of-stdout endpoint on fd >= 3 (or
//! fd 2 via `2>&1`) carries `FdTable::stdout_alias_generation == Some(None)`
//! — "the REAL process stdout snapshotted at dup time". The write arms
//! (`write_fd_endpoint` / `write_output_fd_redirect` /
//! `OutputTarget::ProcessStdoutAt` in redirection.rs) resolved that record
//! through `write_stdout_bytes`, whose thread-local active-capture check
//! re-resolved it to the substitution's own buffer — capturing bytes GNU
//! sends outside. The comsub boundary also left the parent's fd 1 dup record
//! on the child's fresh fd 1. Fixed by `write_real_stdout_uncaptured` plus
//! the fd 1 record clear in `command_substitution_executor`
//! (rubash#368; the function-call fast path's equivalent clear is the
//! #335 residue precedent in embedded_mutations.rs).
//!
//! Expectations byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash) script-file probes (target/probe368: p1, p2, m1).

use std::{fs, path::Path, process::Command};

fn stream_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn run(script: &str) -> (String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (stream_text(&output.stdout), stream_text(&output.stderr))
}

fn run_script_file(body: &str) -> (String, String) {
    let dir = Path::new("target").join(format!("rubash-issue368-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let script = dir.join("case368.sh");
    fs::write(&script, format!("{body}\n")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script)
        .output()
        .expect("run rubash script");
    let _ = fs::remove_dir_all(&dir);
    (stream_text(&output.stdout), stream_text(&output.stderr))
}

#[test]
fn comsub_sees_exec_dupped_stdout_fd() {
    // subst.c:7320 replaces only fd 1; fd 3 keeps the parent's real-stdout
    // binding, so `>&3` escapes the capture (GNU: `A-out` outside, v=[]).
    let (stdout, stderr) = run("exec 3>&1; v=\"$(echo A-out >&3)\"; echo \"v=[$v]\"");
    assert_eq!(stdout, "A-out\nv=[]\n");
    assert_eq!(stderr, "");
}

#[test]
fn comsub_sees_group_dupped_stdout_fd() {
    // Group `3>&1` (open_compound_output_redirects dup) — same class.
    let (stdout, stderr) = run("{ v=\"$(echo D-grp >&3)\"; } 3>&1; echo \"v=[$v]\"");
    assert_eq!(stdout, "D-grp\nv=[]\n");
    assert_eq!(stderr, "");
}

#[test]
fn comsub_stdout_moves_to_group_dupped_fd4() {
    // `1>&4` inside the body: fd 1 := the outer fd 4 (real stdout), so the
    // plain echo escapes while anything on fd 3 stays captured.
    let (stdout, stderr) = run("{ v=\"$(echo E-grp 1>&4)\"; } 4>&1; echo \"v=[$v]\"");
    assert_eq!(stdout, "E-grp\nv=[]\n");
    assert_eq!(stderr, "");
}

#[test]
fn nvm_juggle_progress_outside_value_captured() {
    // The #368 headline reproducer (nvm-exec's `nvm_rc_version` protocol):
    // both halves of the juggle must take effect inside the substitution.
    let (stdout, stderr) = run_script_file(
        "emit() { echo \"progress\"; echo \"value\" >&3; }\n\
         { captured=\"$(emit 3>&1 1>&4)\"; } 4>&1\n\
         echo \"captured=[$captured]\"",
    );
    assert_eq!(stdout, "progress\ncaptured=[value]\n");
    assert_eq!(stderr, "");
}

#[test]
fn nested_comsub_inherits_outer_fd3() {
    // The inner substitution forks from the juggled context and inherits
    // fd 3 = outer stdout; the outer echo (fd 1 -> fd 4) prints outside
    // with the inner's (empty) result.
    let (stdout, stderr) = run_script_file(
        "{ a=\"$(echo $(echo deep >&3) tail)\"; } 3>&1\n\
         echo \"a=[$a]\"",
    );
    assert_eq!(stdout, "deep\na=[tail]\n");
    assert_eq!(stderr, "");
}

#[test]
fn fd3_bound_inside_body_goes_to_capture() {
    // Control (was already correct): `3>&1` applied INSIDE the body binds
    // fd 3 to the capture pipe (redir.c:1170 dups the child's CURRENT fd 1)
    // — the write must be captured, not escape.
    let (stdout, stderr) = run("v=\"$( { echo F-in >&3; } 3>&1 )\"; echo \"v=[$v]\"");
    assert_eq!(stdout, "v=[F-in]\n");
    assert_eq!(stderr, "");
}

#[test]
fn exec_dupped_stderr_fd_still_escapes_capture() {
    // Control: the Stderr-endpoint arm was already correct (#270 family);
    // must not regress — fd 3 := fd 2 routes to stderr outside the capture.
    let (stdout, stderr) = run("exec 3>&2; v=\"$(echo B-err >&3)\"; echo \"v=[$v]\"");
    assert_eq!(stdout, "v=[]\n");
    assert_eq!(stderr, "B-err\n");
}

#[test]
fn closed_fd_reports_bad_descriptor_inside_comsub() {
    // `exec 3>&-` persists; `>&3` inside the substitution is EBADF
    // (redir.c:1170 dup2 failure path), the substitution yields empty.
    let (stdout, stderr) =
        run("exec 3>&1; exec 3>&-; v=\"$(echo closed >&3)\"; echo \"v=[$v]\"; echo \"rc=$?\"");
    assert_eq!(stdout, "v=[]\nrc=0\n");
    assert!(
        stderr.contains("3: Bad file descriptor"),
        "expected EBADF diagnostic, got stderr: {stderr:?}"
    );
}

#[test]
fn fd2_bound_to_stdout_carries_diagnostics_outside() {
    // `exec 2>&1` snapshots fd 1's object onto fd 2; the substitution
    // child's expansion diagnostic goes to fd 2 = real stdout, outside the
    // capture (GNU-verified: `pre` captured, `boom` on stdout).
    let (stdout, stderr) = run("exec 2>&1; v=\"$(echo pre; echo ${nope?boom})\"; echo \"v=[$v]\"");
    assert!(stdout.contains("v=[pre]\n"), "stdout: {stdout:?}");
    assert!(
        stdout.contains(": nope: boom"),
        "diagnostic must reach real stdout, got: {stdout:?}"
    );
    assert_eq!(stderr, "");
}

#[test]
fn chained_exec_dups_visible_in_comsub() {
    // `exec 3>&1; exec 4>&3` — the chain resolves through fd 3's real-
    // stdout object (dup of a dup); `>&4` escapes the capture.
    let (stdout, stderr) = run("exec 3>&1; exec 4>&3; v=\"$(echo chained >&4)\"; echo \"v=[$v]\"");
    assert_eq!(stdout, "chained\nv=[]\n");
    assert_eq!(stderr, "");
}

#[test]
fn nested_function_comsub_keeps_capture_stack_correct() {
    // #335-residue guard under the new fd 1 record clear: the function's
    // nested `$(echo sub)` must read ITS OWN capture (not leak a level up),
    // while the juggled fd 1 -> fd 4 half still escapes.
    let (stdout, stderr) = run_script_file(
        "inner() { echo \"nested-$(echo sub)\"; echo \"ver\" >&3; }\n\
         { N=\"$(inner 3>&1 1>&4)\"; } 4>&1\n\
         echo \"N=[$N]\"",
    );
    assert_eq!(stdout, "nested-sub\nN=[ver]\n");
    assert_eq!(stderr, "");
}
