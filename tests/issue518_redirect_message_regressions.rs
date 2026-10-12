//! Issue rubash#518 — redirection error message fidelity.
//!
//! a) `<&"${X[0]}"` with X set-empty: GNU redirection_expand yields one
//!    anchored empty field (not ambiguous), all_digits("") is vacuously
//!    true but valid_number("") fails → dest -1 → dup2 EBADF, and
//!    redirection_error (redir.c:149-176) echoes the RAW word:
//!    `"${X[0]}": Bad file descriptor`. Rubash leaked the fully-quoted
//!    word's STORAGE_WORD_PREFIX role marker (0x1d) into the diagnostic
//!    and classified the operand as a non-digit word ("ambiguous
//!    redirect" at the gate, "Invalid argument" on the function-call
//!    apply path that bypassed the gate entirely).
//!
//! b) `cmd >> word &` / `cmd >> word` with an unopenable target: GNU
//!    redir.c redirection_error carries the target word in every failed
//!    open diagnostic (`$FIX/b: No such file or directory` ×4 for the
//!    issue's async loop). Rubash's append open propagated a bare OS
//!    error, so the renderer printed strerror text without the word.
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (script-file probes, 2026-10-12; the `sort`-line quote difference the
//! issue excludes is a sort.exe build difference, not a rubash cell).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// a) the quoted empty dup word reports EBADF under the raw word — at top
/// level, and on the function-invocation path (which used to bypass the
/// gate and report `&<0x1d>: Invalid argument`).
#[test]
fn dup_empty_quoted_word_reports_bad_fd_with_raw_word() {
    let (stdout, stderr, _code) =
        rubash("X=; oneread() { r=skip; }; oneread r <&\"${X[0]}\"; echo \"[$r]\"");
    assert_eq!(stderr, "bash: line 1: \"${X[0]}\": Bad file descriptor\n");
    // The command never runs, so `r` stays unset (GNU: `[]`).
    assert_eq!(stdout, "[]\n");
}

/// a-guard: `>&"${X[0]}"` (dup output form) takes the same shape.
#[test]
fn dup_output_empty_quoted_word_reports_bad_fd() {
    let (_stdout, stderr, _code) = rubash("X=; cat >&\"${X[0]}\"");
    assert!(stderr.ends_with("\"${X[0]}\": Bad file descriptor\n"));
    assert!(
        !stderr.contains('\u{1d}'),
        "0x1d marker leaked into the diagnostic: {stderr:?}"
    );
}

/// a-guard: no marker byte may appear in any redirect diagnostic.
#[test]
fn redirect_diagnostics_carry_no_marker_bytes() {
    let (_stdout, stderr, _code) = rubash("X=; cat <&\"${X[0]}\"");
    assert!(
        !stderr.contains('\u{1d}') && !stderr.contains('\u{13}'),
        "marker byte leaked: {stderr:?}"
    );
}

/// b) a foreground append open failure carries the target word (the
/// open_command_output_target routing pass, redirection.rs).
#[test]
fn foreground_append_open_error_keeps_word() {
    let (_stdout, stderr, _code) = rubash(": >> \"/nonexistent_rubash_518_dir/b\"");
    assert_eq!(
        stderr,
        "bash: line 1: /nonexistent_rubash_518_dir/b: No such file or directory\n"
    );
}

/// b) the issue's async repro: every background append failure carries the
/// word (GNU prints the word on all four lines).
#[test]
fn background_append_open_errors_keep_word() {
    let (_stdout, stderr, _code) = rubash(
        "FIX=/nonexistent_rubash_518_dir; : > '$FIX/b'; \
         for i in 1 2 3; do echo \\$i >> '$FIX/b' & done; wait",
    );
    assert_eq!(
        stderr.matches("$FIX/b: No such file or directory").count(),
        4,
        "stderr={stderr:?}"
    );
}

/// b-guard: external append (`cat`) keeps the word too.
#[test]
fn external_append_open_error_keeps_word() {
    let (_stdout, stderr, _code) = rubash("cat < /dev/null >> \"/nonexistent_rubash_518_dir/b\"");
    assert_eq!(
        stderr,
        "bash: line 1: /nonexistent_rubash_518_dir/b: No such file or directory\n"
    );
}
