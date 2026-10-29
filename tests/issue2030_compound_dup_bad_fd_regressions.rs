//! rubash#2030 — a dup whose source fd is unbound inside a COMPOUND
//! binding reports `N: Bad file descriptor` and the binding level dies.
//!
//! GNU redir.c:
//! - do_redirection_internal, case r_duplicating_input (redir.c:1179):
//!   `dup2 (redir_fd, redirector) < 0` returns errno (EBADF for a closed
//!   source); do_redirections (redir.c:255-260) reports the failure and
//!   abandons the command without running the body.
//! - redirection_error's EBADF arm (redir.c:149-158) names the SOURCE fd
//!   for the duplicating/move instructions — WSL GNU 5.3.20: `done <&3`
//!   with fd 3 closed prints `3: Bad file descriptor`, and `done 5<&3`
//!   ALSO names the source (`3:`), not the redirector.
//! - The failure propagates through a pipeline stage as the stage's
//!   status; the consuming stage still runs (`… | wc -l` counts 0).
//!
//! rubash's compound dup preflight used to cover only the OUTPUT side
//! (rubash#263/#266); an input dup of an unbound fd fell through `let _
//! = dup_input` and the level silently read the ambient descriptor —
//! `{ while read x; …; done <&3; }` read the pipeline's stdin and
//! printed 9 where GNU prints 0. The fix generalizes the preflight to
//! one left-to-right walk over BOTH sides (compound_dup_bad_fd), wired
//! before any bind so the abandonment leaves no half-applied state, at
//! the compound wrapper, with_numbered_fd_bindings and the exec face.
//!
//! All expectations measured against GNU bash 5.3.20 with script files
//! (WSL /usr/local/bin/bash, 2026-10-29).

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2030-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = tempdir(tag);
    let case = dir.join("case.sh");
    std::fs::write(&case, script).expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg(&case)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &dir)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue2030_loop_dup_from_unbound_fd_reports_and_dies() {
    // The issue's reproducer, fed by a pipe instead of `seq` so the test
    // only needs shell builtins: the level must die (no `got:` lines,
    // stage status 1) instead of silently reading the pipe.
    let (stdout, stderr, code) = run(
        "loop",
        "printf 'a\\nb\\n' | { while read x; do echo \"got:$x\"; done <&3; }\necho \"rc=$?\"\n",
    );
    assert_eq!(stdout, "rc=1\n", "the level dies: stdout: {stdout}");
    assert!(
        stderr.contains("3: Bad file descriptor"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue2030_loop_dup_pipeline_consumer_still_counts() {
    // The issue's exact shape: GNU prints 0 (the dead stage feeds wc
    // nothing) — not the pre-fix 9 (silent fallback to the pipe).
    let (stdout, stderr, code) = run(
        "pipeline",
        "printf '1\\n2\\n3\\n4\\n5\\n6\\n7\\n8\\n9\\n' | { while read x; do echo \"$x\"; done <&3; } | { read n; echo \"lines=$n\"; }\necho \"rc=$?\"\n",
    );
    // The dead stage produces nothing; the consumer's `read` sees EOF and
    // the pipeline status is the consumer group's (0, its echo's status).
    assert_eq!(stdout, "lines=\nrc=0\n", "stdout: {stdout}");
    assert!(
        stderr.contains("3: Bad file descriptor"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue2030_brace_group_dup_unbound_fd() {
    let (stdout, stderr, code) = run("group", "{ echo body-ran; } <&5\necho \"rc=$?\"\n");
    assert!(!stdout.contains("body-ran"), "stdout: {stdout}");
    assert_eq!(stdout, "rc=1\n", "stdout: {stdout}");
    assert!(
        stderr.contains("5: Bad file descriptor"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue2030_move_dup_unbound_fd_names_source() {
    // GNU redirection_error's EBADF arm names the SOURCE fd for the move
    // form too: `done 5<&3` with fd 3 closed prints `3: Bad file
    // descriptor` (not `5:`).
    let (stdout, stderr, code) = run("move", "while :; do :; done 5<&3\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n", "stdout: {stdout}");
    assert!(
        stderr.contains("3: Bad file descriptor"),
        "stderr: {stderr}"
    );
    assert!(
        !stderr.contains("5: Bad file descriptor"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue2030_bound_dup_still_works() {
    // Control: a dup whose source IS open binds the level normally.
    let (stdout, stderr, code) = run(
        "bound",
        "printf 'a\\nb\\n' > dupf.txt\n{ exec 3< dupf.txt; while read x; do echo \"[$x]\"; done <&3; }\necho \"rc=$?\"\n",
    );
    assert_eq!(stdout, "[a]\n[b]\nrc=0\n", "stdout: {stdout}");
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue2030_move_from_explicitly_closed_fd_fails() {
    // `exec 4<&-` closes fd 4; the scoped move `<&4-` then dups a closed
    // source. GNU reports `4: Bad file descriptor` (plus its internal
    // `redirection error: cannot duplicate fd` prolog for the move undo,
    // redir.c add_undo_redirect) and the body does not run.
    let (stdout, stderr, code) = run(
        "closedmove",
        "printf 'a\\n' > dupf.txt\nexec 4< dupf.txt\nexec 4<&-\n{ echo x; } <&4-\necho \"rc=$?\"\n",
    );
    assert!(
        !stdout.contains("x\nrc="),
        "the body must not run: stdout: {stdout}"
    );
    assert_eq!(stdout, "rc=1\n", "stdout: {stdout}");
    assert!(
        stderr.contains("4: Bad file descriptor"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue2030_exec_face_dup_unbound_fd() {
    // `exec cat <&9`: GNU exec_builtin fails do_redirections before the
    // execve — `9: Bad file descriptor`, status 1, the shell continues.
    let (stdout, stderr, code) = run("exec", "exec cat <&9\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n", "stdout: {stdout}");
    assert!(
        stderr.contains("9: Bad file descriptor"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue2030_first_failing_dup_in_list_order_wins() {
    // GNU do_redirections walks the list in order and stops at the first
    // failing step: `2>&4 <&3` reports `4:`, never `3:`.
    let (stdout, stderr, code) = run("order", "{ echo x; } 2>&4 <&3\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n", "stdout: {stdout}");
    assert!(
        stderr.contains("4: Bad file descriptor"),
        "stderr: {stderr}"
    );
    assert!(
        !stderr.contains("3: Bad file descriptor"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}
