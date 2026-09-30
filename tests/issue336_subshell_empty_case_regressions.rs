//! Issue rubash#336 regressions: a subshell-wrapped empty case —
//! `( case x in esac )` — failed to parse ("unexpected end of file from
//! `('"), while GNU Bash 5.3.0 runs it (rc=0).
//!
//! Root cause: the token-level group scanners (matching_subshell_end in
//! parser/subshell_command.rs, the function-body paren scanner in
//! parser/function_command.rs, the coproc subshell-body scanner in
//! parser/coproc_command.rs, and the procsub target scanner in
//! parser/process_substitution.rs) recognize an `esac` that closes a
//! case only at a reserved-word boundary
//! (`command_boundary_keyword_allowed`). But `in` is NOT a reserved-word
//! boundary — GNU reserved_word_acceptable (parse.y:5898) has no IN —
//! and the empty-case production `case WORD newline_list IN newline_list
//! ESAC` (parse.y:1037) is closed by `esac` DIRECTLY after `in` via the
//! one special case in parse.y:3428-3441 special_case_tokens()
//! (esacs_needed_count && last_read_token == IN -> return ESAC). GNU
//! never reads an after-`in` `esac` as pattern text (the rule-4 refusals
//! at parse.y:3184-3186 apply only after `|' / `('), so
//! parser/support.rs `is_case_end_keyword` now short-circuits the
//! lookahead heuristic for that position and the scanners admit it via
//! `follows_case_in_keyword`.
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-28, matrix under
//! target/issue-suites/results/resid21/p1..p20 in the lane worktree).

use std::io::Write;
use std::process::{Command, Stdio};

fn rubash_stdin(script: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash stdin");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(script.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("collect stdin run");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

// ---------------------------------------------------------------------------
// The wrapper matrix: every enclosing compound that scans over the empty
// case. GNU 5.3.0 runs each with rc=0 and the echoed marker on stdout.
// ---------------------------------------------------------------------------

#[test]
fn subshell_empty_case_parses() {
    let (stdout, stderr, code) = rubash_stdin("( case x in esac )\necho ok\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "ok\n");
    assert!(stderr.is_empty(), "stderr: {stderr}");
}

#[test]
fn subshell_empty_case_with_newline_before_esac() {
    let (stdout, stderr, code) = rubash_stdin("( case x in\nesac )\necho ok\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "ok\n");
}

#[test]
fn subshell_empty_case_in_pipeline() {
    let (stdout, stderr, code) = rubash_stdin("( case x in esac ) | cat\necho ok\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "ok\n");
}

#[test]
fn function_body_paren_form_empty_case() {
    let (stdout, stderr, code) = rubash_stdin("f() ( case x in esac )\nf\necho ok\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "ok\n");
}

#[test]
fn process_substitution_empty_case() {
    let (stdout, stderr, code) = rubash_stdin("echo <(case x in esac) >/dev/null && echo ok\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "ok\n");
}

#[test]
fn coproc_subshell_body_empty_case() {
    let (stdout, stderr, code) = rubash_stdin("coproc ( case x in esac )\nwait\necho ok\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "ok\n");
}

#[test]
fn subshell_empty_case_inside_case_clause() {
    let (stdout, stderr, code) = rubash_stdin("case y in b) ( case x in esac ) ;; esac\necho ok\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "ok\n");
}

#[test]
fn subshell_empty_case_inside_if_and_loop_bodies() {
    let (stdout, stderr, code) = rubash_stdin(
        "if :; then ( case x in esac ); fi\nwhile :; do ( case x in esac ); break; done\necho ok\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "ok\n");
}

#[test]
fn loop_body_empty_case_inside_subshell() {
    let (stdout, stderr, code) =
        rubash_stdin("( while :; do case x in esac; break; done )\necho ok\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "ok\n");
}

#[test]
fn comsub_subshell_empty_case_captures_empty() {
    let (stdout, stderr, code) = rubash_stdin("v=$( (case x in esac) )\necho \"v=[$v]\"\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "v=[]\n");
}

// ---------------------------------------------------------------------------
// Negative guards (also GNU-verified): the after-`in` recognition must
// not turn word-list `esac` into a closer, and the invalid shapes keep
// reporting the paren as the offending token.
// ---------------------------------------------------------------------------

#[test]
fn for_word_list_esac_stays_a_word() {
    let (stdout, stderr, code) = rubash_stdin("for i in esac; do echo \"w=$i\"; done\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "w=esac\n");
}

#[test]
fn empty_case_then_stray_paren_still_errors_at_paren() {
    let (stdout, stderr, code) = rubash_stdin("case x in esac) foo ;; esac\n");
    assert_eq!(code, Some(2));
    assert!(stdout.is_empty(), "stdout: {stdout}");
    assert!(
        stderr.contains("syntax error near unexpected token `)'"),
        "stderr: {stderr}"
    );
}

#[test]
fn esac_after_pipe_stays_pattern_text() {
    // GNU: rc=0, no output (the a|esac) clause does not match x).
    let (stdout, stderr, code) = rubash_stdin("case x in a|esac) ;; esac\necho fin\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "fin\n");
}
