//! Issues rubash#339 / rubash#340 — process-substitution parse admission.
//!
//! #339: `a=(<(echo e))` died as `syntax error near unexpected token \`<'`
//! (rc=2) and `cat 2>(echo x)` as `syntax error near unexpected token \`('`
//! — both accepted by GNU. Root causes fixed:
//!   * find_unquoted_ctrl_op (parser/token_actions.rs) treated an unquoted
//!     `<(`/`>(` inside a compound-assignment value as a control operator;
//!     GNU parse_compound_assignment (parse.y:7140) reads each element via
//!     read_token, and `<`+`(` is a procsub WORD (read_token parse.y:3794
//!     hands it to read_token_word, whose shellexp arm parse.y:5490-5524
//!     consumes the `(list)` into the token). A bare `<`/`>` stays an
//!     operator (`a=(<x)` still errors, GNU-verified).
//!   * The lexer's digit arms made `2>`/`3<` into redirect OPERATORS even
//!     when `(` followed; GNU's NUMBER decision (parse.y:5729-5738) only
//!     fires when the word ENDED at the `<`/`>`, which `>(` prevents — the
//!     digits are word text glued to the substitution result.
//!   * process_substitutions_in_word_with_raw found only word-initial
//!     substitutions; GNU extract_process_subst (subst.c:1311, reached from
//!     expand_word_internal cases '<'/'>', subst.c:11349-11378) extracts
//!     UNQUOTED spans ANYWHERE in the word (`echo p<(echo x)q`).
//!
//! #340: `cat <& <(echo x)` executed the substitution and duped from the
//! temp file (rc=0). GNU r_duplicating_input_word (redir.c:784-843) rejects
//! the non-digit expansion with AMBIGUOUS_REDIRECT reporting the LITERAL
//! word (redirection_error re-expands with W_NOCOMSUB|W_NOPROCSUB,
//! redir.c:186-190): `<(echo x): ambiguous redirect`, rc=1. `2>& <(...)`
//! same; the exception is `>&WORD` with redirector 1 (r_err_and_out,
//! redir.c:832-838), which opens the expanded filename and succeeds.
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-28), except where a
//! documented platform divergence is noted (temp-file vs /dev/fd path form
//! and fd lifecycle, tracked by rubash#355).

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

/// #339 repro A: array assignment with a procsub element parses and the
/// element materializes (GNU: `cat: /dev/fd/63: No such file or directory`
/// because the fd is already closed; rubash's temp-file procsub keeps the
/// bytes readable — the #355 path-form/lifecycle class, not parse).
#[test]
fn procsub_array_assignment_parses_and_materializes() {
    let (stdout, stderr, code) = rubash("a=(<(echo e))\ncat \"${a[0]}\"");
    assert!(!stderr.contains("syntax error"), "stderr: {stderr}");
    assert_ne!(code, Some(2));
    assert_eq!(stdout, "e\n");
}

/// #339 guard: a bare `<` inside a compound assignment is still the
/// redirection operator — GNU rejects `a=(<x)` with `unexpected token \`<'`.
#[test]
fn compound_assignment_bare_redirect_still_rejected() {
    let (stdout, stderr, code) = rubash("a=(<x)");
    assert_eq!(stdout, "");
    assert_ne!(code, Some(0));
    assert!(
        stderr.contains("syntax error near unexpected token `<'"),
        "stderr: {stderr}"
    );
}

/// #339 repro B: `2>(echo x)` is one WORD — `2` glued to the substitution
/// result. GNU: stderr `cat: 2/dev/fd/63: No such file or directory`, the
/// `x` lands on stdout via the substitution, rc=1. rubash matches the
/// message shape and the `x` (rc propagation through the async output
/// substitution is a residual, 0 vs 1).
#[test]
fn digit_prefixed_output_procsub_is_one_word() {
    let (stdout, stderr, code) = rubash("cat 2>(echo x)");
    assert!(!stderr.contains("syntax error"), "stderr: {stderr}");
    assert_ne!(code, Some(2));
    assert_eq!(stdout, "x\n");
    assert!(
        stderr.contains("cat: 2/") && stderr.contains(": No such file or directory"),
        "stderr: {stderr}"
    );
}

/// #339: mid-word procsub splices into the word (`echo p<(echo x)q`).
#[test]
fn mid_word_procsub_splices_into_word() {
    let (stdout, stderr, code) = rubash("v=$(echo p<(echo x)q); printf '%s' \"$v\"");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert!(stdout.starts_with("p/"), "stdout: {stdout}");
    assert!(stdout.ends_with('q'), "stdout: {stdout}");
    assert!(stdout.contains(".tmp"), "stdout: {stdout}");
}

/// #339 guard: a QUOTED `<(` is data — `echo "<(echo x)"` prints the literal
/// text (GNU subst.c:11349-11378 quoted branch).
#[test]
fn quoted_procsub_text_stays_literal() {
    let (stdout, _, code) = rubash(r#"echo "<(echo x)""#);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "<(echo x)\n");
}

/// #340: `cat <& <(echo x)` is an ambiguous redirect; the command never
/// runs (no `x` on stdout), rc=1, and the diagnostic carries the LITERAL
/// substitution text.
#[test]
fn dup_input_word_procsub_is_ambiguous_redirect() {
    let (stdout, stderr, code) = rubash("cat <& <(echo x)");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(1));
    assert!(
        stderr.contains("<(echo x): ambiguous redirect"),
        "stderr: {stderr}"
    );
}

/// #340: `2>& <(echo x)` — same r_duplicating_output_word rejection.
#[test]
fn stderr_dup_word_procsub_is_ambiguous_redirect() {
    let (stdout, stderr, code) = rubash("cat 2>& <(echo x)");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(1));
    assert!(
        stderr.contains("<(echo x): ambiguous redirect"),
        "stderr: {stderr}"
    );
}

/// #340 exception: `>&WORD` with redirector 1 is r_err_and_out (GNU probe:
/// rc=0, silent) — never a syntax error.
#[test]
fn stdout_dup_word_procsub_err_and_out_runs() {
    let (stdout, stderr, code) = rubash("cat >& <(echo x)");
    assert_eq!(code, Some(0));
    assert!(!stderr.contains("ambiguous"), "stderr: {stderr}");
    assert!(!stderr.contains("syntax error"), "stderr: {stderr}");
    // Residual (#355 class): rubash's temp-file procsub leaks the body
    // output to the child's stdout; GNU writes it to the substitution pipe.
    assert!(stdout == "" || stdout == "x\n");
}

/// #340 guard: the ordinary dup forms keep working.
#[test]
fn ordinary_dup_redirects_still_work() {
    let (stdout, _, code) = rubash("printf hi | { cat <&0; }");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "hi");

    let (stdout, stderr, code) = rubash("cat <&9");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(1));
    assert!(
        stderr.contains("9: Bad file descriptor"),
        "stderr: {stderr}"
    );
}

/// #340 guard: a missing redirect target next to an operator is still a
/// syntax error naming that operator — the procsub exemption must not
/// swallow the general rule (`foo >& |`).
#[test]
fn missing_redirect_target_still_syntax_error() {
    let (stdout, stderr, code) = rubash("cat >& |");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(2));
    assert!(
        stderr.contains("syntax error near unexpected token `|'"),
        "stderr: {stderr}"
    );
}
