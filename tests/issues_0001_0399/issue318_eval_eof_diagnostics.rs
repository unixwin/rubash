//! Issue rubash#318 regression: eval-family EOF diagnostics must report the
//! eval call-site-continued lines GNU reports, not the function-definition
//! or post-function lines.
//!
//! GNU mechanics (all pinned byte-for-byte vs WSL GNU Bash 5.3.0, probes in
//! target/fix16/m*.sh of the fix16 lane):
//! - evalstring.c:345-357: eval re-parses its string as fresh input
//!   continuing the caller's line counter (`line_number--`, no
//!   SEVAL_RESETLINE), so eval-string line i sits at script line
//!   caller_line + i - 1.
//! - A subshell `(` after body text, immediately followed by newline or end
//!   of string, is a comsub-body yyparse error (parse.y:4549):
//!   `syntax error near unexpected token \`newline' while looking for
//!   matching \`)'` (parse.y:6858-6859, shell_eof_token == ')' from
//!   parse.y:4519) plus the offending source line (parse.y:6865), at the
//!   `(' line; non-interactive shells exit 1 through FORCE_EOF
//!   (parse.y:4588-4596) and run nothing after the eval.
//! - Every other unclosed `$(` (empty body at EOF, body that never closed)
//!   keeps the plain `unexpected EOF while looking for matching \`)'` at one
//!   past the last input line of the string, eval returns 2 and the script
//!   continues (parse.y:4576-4587 / 6891).
//! - An unclosed `name=(` compound list in the eval string is the
//!   parse_compound_assignment EOF family: eval returns exit 1
//!   (parse.y:7140-7152), like the script-level path.

use std::io::Write;
use std::process::Command;

/// Run `script` as a script FILE (line numbers are physical file lines) and
/// return (stdout, stderr-tail-lines-normalized, rc). The normalization
/// strips the invocation-dependent `<path>: ` prolog so assertions only see
/// GNU-identical text.
fn run_script(script: &str) -> (String, Vec<String>, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i318-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&path).expect("create probe");
    file.write_all(script.as_bytes()).expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .output()
        .expect("run rubash");
    let stderr = String::from_utf8_lossy(&output.stderr)
        .lines()
        .map(|line| {
            // `<path>: eval: line N: msg` / `<path>: eval: line N: `src`'
            match line.find(": eval: ") {
                Some(idx) => line[idx + 2..].to_string(),
                None => line.to_string(),
            }
        })
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr,
        output.status.code(),
    )
}

/// `echo $(echo (` — the bare-`(` yacc class at the `(' line, abort rc 1,
/// nothing after the eval runs. GNU:
/// `eval: line 3: syntax error near unexpected token `newline' while
///  looking for matching `)'` + `eval: line 3: `echo $(echo ('`.
#[test]
fn bare_paren_in_comsub_reports_yacc_error_and_aborts() {
    let (stdout, stderr, rc) =
        run_script("f() {\n  echo start\n  eval \"echo \\$(echo (\"\n}\nf\necho after\n");
    assert_eq!(stdout, "start\n", "stdout");
    assert_eq!(
        stderr,
        vec![
            "eval: line 3: syntax error near unexpected token `newline' while looking for matching `)'".to_string(),
            "eval: line 3: `echo $(echo ('".to_string(),
        ],
        "stderr"
    );
    assert_eq!(rc, Some(1));
}

/// Multi-line eval string with the comsub on string line 2: the yacc error
/// and offending line name line caller+1; the complete first string line
/// already ran (GNU probe m1).
#[test]
fn bare_paren_after_executed_prefix_reports_comsub_line() {
    let (stdout, stderr, rc) =
        run_script("f() {\n  echo start\n  eval \"echo a\n  echo \\$(echo (\"\n}\nf\n");
    assert_eq!(stdout, "start\na\n", "stdout");
    assert_eq!(
        stderr,
        vec![
            "eval: line 4: syntax error near unexpected token `newline' while looking for matching `)'".to_string(),
            "eval: line 4: `  echo $(echo ('".to_string(),
        ],
        "stderr"
    );
    assert_eq!(rc, Some(1));
}

/// `echo $(` — empty comsub body at end of input keeps the plain wording at
/// one past the string's last line, eval returns 2, the script continues.
#[test]
fn empty_comsub_body_reports_one_past_last_line_and_continues() {
    let (stdout, stderr, rc) =
        run_script("f() {\n  echo start\n  eval \"echo \\$(\"\n}\nf\necho after\n");
    assert_eq!(stdout, "start\nafter\n", "stdout");
    assert_eq!(
        stderr,
        vec!["eval: line 4: unexpected EOF while looking for matching `)'".to_string()],
        "stderr"
    );
    // eval returned 2 but the script continued; the trailing `echo after`
    // owns the script exit status (GNU continues past this class).
    assert_eq!(rc, Some(0));
}

/// Multi-line string, `$(` on the last line: plain wording at one past the
/// last line (caller + newlines + 1); earlier complete string lines ran
/// (GNU probe m9: `a`, `b` printed, line 5, rc 2).
#[test]
fn empty_body_multiline_string_reports_after_last_line() {
    let (stdout, stderr, rc) =
        run_script("f() {\n  eval \"echo a\necho b\necho \\$(\"\n}\nf\necho after\n");
    assert_eq!(stdout, "a\nb\nafter\n", "stdout");
    assert_eq!(
        stderr,
        vec!["eval: line 5: unexpected EOF while looking for matching `)'".to_string()],
        "stderr"
    );
    // Script continues; trailing `echo after` owns the exit status.
    assert_eq!(rc, Some(0));
}

/// `echo $(echo hi` — body consumed input but never closed: plain wording at
/// one past the last line; the failing command itself runs nothing (GNU
/// probe m5: only `after` printed, rc 0 — the script continues past eval's
/// status 2).
#[test]
fn consumed_body_never_closed_reports_after_last_line() {
    let (stdout, stderr, rc) =
        run_script("f() {\n  eval \"echo \\$(echo hi\necho b\"\n}\nf\necho after\n");
    assert_eq!(stdout, "after\n", "stdout");
    assert_eq!(
        stderr,
        vec!["eval: line 4: unexpected EOF while looking for matching `)'".to_string()],
        "stderr"
    );
    assert_eq!(rc, Some(0));
}

/// `echo $( (` — a `(` directly after the comsub opener is a matched-pair
/// EOF (GNU probe m8), NOT the yacc bare-paren class: plain wording at one
/// past the last line, rc 2.
#[test]
fn paren_directly_after_comsub_open_is_plain_eof() {
    let (stdout, stderr, rc) = run_script("f() {\n  eval \"echo \\$( (\"\n}\nf\n");
    assert_eq!(stdout, "", "stdout");
    assert_eq!(
        stderr,
        vec!["eval: line 3: unexpected EOF while looking for matching `)'".to_string()],
        "stderr"
    );
    assert_eq!(rc, Some(2));
}

/// `x=(` inside eval — compound-assignment EOF family: eval returns 1 (GNU
/// probe n1), message unchanged.
#[test]
fn compound_assignment_eof_returns_one() {
    let (stdout, stderr, rc) = run_script("f() {\n  echo start\n  eval \"x=(\"\n}\nf\n");
    assert_eq!(stdout, "start\n", "stdout");
    assert_eq!(
        stderr,
        vec!["eval: line 3: unexpected EOF while looking for matching `)'".to_string()],
        "stderr"
    );
    assert_eq!(rc, Some(1));
}

// ---------------------------------------------------------------------------
// rubash#333 (2026-09-28 snaprec lane): the #318 abort model, re-verified
// against WSL GNU Bash 5.3.0 (probe matrix p1-p28 under
// target/snaprec-probe/). GNU's taxonomy has THREE abort classes for an
// eval-string parse error:
//
//   1. comsub-body token error (`$(echo (`) and posix compound-assignment
//      failures (`x=(`, `x=((`) jump FORCE_EOF — the shell exits 1 with no
//      `command'-builtin gate;
//   2. everything else fails the MAIN yyparse into evalstring.c:584-600,
//      whose only exit is posix + bare eval/source (executing_command_
//      builtin == 0) — ERREXIT exit 2 before the || branch runs;
//   3. in every other case the error is CONTAINED to the eval string:
//      push_stream/pop_stream clear EOF_Reached per stream (parse.y:1908/
//      1915), so the outer script's reader always continues.
//
// The 2026-09-29 #318 commit implemented class 1 and the class-3 wording,
// but aborted the outer script after the current command group for every
// posix script (mark_parse_error leaking into the grouped driver), which
// killed errors8.sub lines 8+ after `command eval '( '` — GNU prints
// ok 1..ok 8 there (errors.right:244-259).
// ---------------------------------------------------------------------------

/// The errors8.sub line-6 shape: `command eval '( '` in a posix script.
/// GNU continues the script (`command` sets executing_command_builtin,
/// suppressing the posix ERREXIT of evalstring.c:588-595) and reports the
/// bash-5.3 compoundcmd EOF wording (y.tab.c:9258).
#[test]
fn command_eval_plain_paren_posix_continues() {
    let (stdout, stderr, rc) =
        run_script("set -o posix\ncommand eval '( ' || echo ok 1\necho survived\n");
    assert_eq!(stdout, "ok 1\nsurvived\n", "stdout");
    assert_eq!(
        stderr,
        vec![
            "eval: line 3: syntax error: unexpected end of file from `(' command on line 2"
                .to_string()
        ],
        "stderr"
    );
    assert_eq!(rc, Some(0));
}

/// Bare `eval '( '` in a posix script takes the ERREXIT exit: status 2,
/// nothing after the eval runs — not even the || branch (GNU probe p3).
#[test]
fn bare_eval_plain_paren_posix_exits_two() {
    let (stdout, stderr, rc) = run_script("set -o posix\neval '( ' || echo ok 1\necho after\n");
    assert_eq!(stdout, "", "stdout");
    assert_eq!(
        stderr,
        vec![
            "eval: line 3: syntax error: unexpected end of file from `(' command on line 2"
                .to_string()
        ],
        "stderr"
    );
    assert_eq!(rc, Some(2));
}

/// Non-posix bare `eval '( '`: contained — the || branch and the rest of
/// the script run, eval yields 2 (GNU probe p7a).
#[test]
fn bare_eval_plain_paren_non_posix_continues() {
    let (stdout, stderr, rc) = run_script("eval '( ' || echo ok 1\necho after\n");
    assert_eq!(stdout, "ok 1\nafter\n", "stdout");
    assert_eq!(
        stderr,
        vec![
            "eval: line 2: syntax error: unexpected end of file from `(' command on line 1"
                .to_string()
        ],
        "stderr"
    );
    assert_eq!(rc, Some(0));
}

/// The comsub EOF class keeps the plain wording but obeys the same
/// command/posix gate: `command eval '$( '` continues (GNU probe p9a),
/// bare `eval '$( '` in posix exits 2 with nothing after (probe p5a).
#[test]
fn comsub_eof_posix_gate_follows_command_builtin() {
    let (stdout, stderr, rc) =
        run_script("set -o posix\ncommand eval '$( ' || echo ok 1\necho survived\n");
    assert_eq!(stdout, "ok 1\nsurvived\n", "stdout");
    assert_eq!(
        stderr,
        vec!["eval: line 3: unexpected EOF while looking for matching `)'".to_string()],
        "stderr"
    );
    assert_eq!(rc, Some(0));

    let (stdout, stderr, rc) = run_script("set -o posix\neval '$( ' || echo ok 1\necho after\n");
    assert_eq!(stdout, "", "stdout");
    assert_eq!(
        stderr,
        vec!["eval: line 3: unexpected EOF while looking for matching `)'".to_string()],
        "stderr"
    );
    assert_eq!(rc, Some(2));
}

/// A plain `(` after body text at the main level is a token error naming
/// the NEXT token — `echo (` names `newline' (the funcdef shift consumed
/// the paren), `echo ((` names `(', `echo (hi` names `hi' — WITHOUT the
/// "while looking for matching" suffix (shell_eof_token is 0 outside a
/// comsub, y.tab.c:9208-9215). Contained abort (GNU probes p14/p16/p21).
#[test]
fn main_level_token_error_names_next_token() {
    let (stdout, stderr, rc) = run_script("eval 'echo (' || echo ok 1\necho after\n");
    assert_eq!(stdout, "ok 1\nafter\n", "stdout");
    assert_eq!(
        stderr,
        vec![
            "eval: line 1: syntax error near unexpected token `newline'".to_string(),
            "eval: line 1: `echo ('".to_string(),
        ],
        "stderr"
    );
    assert_eq!(rc, Some(0));

    let (stdout, stderr, rc) = run_script("eval 'echo ((' || echo ok 1\necho after\n");
    assert_eq!(stdout, "ok 1\nafter\n", "stdout");
    assert_eq!(
        stderr,
        vec![
            "eval: line 1: syntax error near unexpected token `('".to_string(),
            "eval: line 1: `echo (('".to_string(),
        ],
        "stderr"
    );
    assert_eq!(rc, Some(0));

    let (stdout, stderr, rc) = run_script("eval 'echo (hi' || echo ok 1\necho after\n");
    assert_eq!(stdout, "ok 1\nafter\n", "stdout");
    assert_eq!(
        stderr,
        vec![
            "eval: line 1: syntax error near unexpected token `hi'".to_string(),
            "eval: line 1: `echo (hi'".to_string(),
        ],
        "stderr"
    );
    assert_eq!(rc, Some(0));
}

/// A plain subshell `(` in command position mid-string: complete prefix
/// lines run, the compoundcmd wording reports one past the last input
/// line, the script continues (GNU probe p25).
#[test]
fn plain_subshell_mid_string_runs_prefix_and_continues() {
    let (stdout, stderr, rc) = run_script("eval 'echo a\n( hi' || echo ok 1\necho after\n");
    assert_eq!(stdout, "a\nok 1\nafter\n", "stdout");
    assert_eq!(
        stderr,
        vec![
            "eval: line 3: syntax error: unexpected end of file from `(' command on line 2"
                .to_string()
        ],
        "stderr"
    );
    assert_eq!(rc, Some(0));
}

/// Compound-assignment failures in posix mode die rc 1 with no `command'
/// gate (parse.y:7173-7179): both `x=(` (probe p13) and the token-error
/// `x=((` (probe p24) exit before the || branch runs.
#[test]
fn compound_assignment_posix_failures_exit_one() {
    let (stdout, stderr, rc) = run_script("set -o posix\neval 'x=(' || echo ok 1\necho after\n");
    assert_eq!(stdout, "", "stdout");
    assert_eq!(
        stderr,
        vec!["eval: line 2: unexpected EOF while looking for matching `)'".to_string()],
        "stderr"
    );
    assert_eq!(rc, Some(1));

    let (stdout, stderr, rc) = run_script("set -o posix\neval 'x=((' || echo ok 1\necho after\n");
    assert_eq!(stdout, "", "stdout");
    assert_eq!(
        stderr,
        vec![
            "eval: line 2: syntax error near unexpected token `('".to_string(),
            "eval: line 2: `x=(('".to_string(),
        ],
        "stderr"
    );
    assert_eq!(rc, Some(1));

    // The `command' prefix does NOT suppress this class (GNU probe p13).
    let (stdout, _stderr, rc) =
        run_script("set -o posix\ncommand eval 'x=(' || echo ok 1\necho after\n");
    assert_eq!(stdout, "", "stdout");
    assert_eq!(rc, Some(1));
}

/// `(( ` and `$(( ` arithmetic roots keep the generic close-char report at
/// the OPEN line — the #318 classifier must not claim them (GNU probes
/// p27/p28; both contained, wording unchanged from the generic path).
#[test]
fn arithmetic_roots_stay_generic() {
    let (stdout, stderr, rc) = run_script("eval '(( ' || echo ok 1\necho after\n");
    assert_eq!(stdout, "ok 1\nafter\n", "stdout");
    assert_eq!(
        stderr,
        vec!["eval: line 1: unexpected EOF while looking for matching `)'".to_string()],
        "stderr"
    );
    assert_eq!(rc, Some(0));

    let (stdout, stderr, rc) = run_script("eval '$(( ' || echo ok 1\necho after\n");
    assert_eq!(stdout, "ok 1\nafter\n", "stdout");
    assert_eq!(
        stderr,
        vec!["eval: line 1: unexpected EOF while looking for matching `)'".to_string()],
        "stderr"
    );
    assert_eq!(rc, Some(0));
}
