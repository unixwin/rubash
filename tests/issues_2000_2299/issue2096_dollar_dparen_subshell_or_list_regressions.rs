//! Issue rubash#2096 — word-level `$(( cmd ) || cmd)` (a command
//! substitution of a subshell-or-list) was classified as arithmetic at six
//! sibling scan sites, and the `))`-pair hunt in `skip_arith_paren` then
//! swallowed past the enclosing quote to end of input. The token-start
//! classification got the `dollar_double_paren_is_arithmetic` gate in
//! rubash#512; the dquoted / mid-word / `${}`-body / nested-comsub sites did
//! not, so nvm's `ALIAS_TEMP="$( ( ... ) || nvm_echo)"` (nvm.sh:1336,
//! reprinted by the respawn prelude as the legal spaceless `$(( ... )` form)
//! killed every respawned background child: `nvm ls` printed six
//! ``syntax error near unexpected token `)' `` lines per invocation and lost
//! the default-alias output.
//!
//! GNU ground truth (vendored bash.git @9c465866, Bash-5.3 patch 20):
//! parse.y:4451 `parse_comsub()` peeks `(` after `$(` and hands the word to
//! `parse_matched_pair(..., P_ARITH)` — a plain balanced nesting match
//! (parse.y:3898 `count = 1`, 3998-4005 increment/decrement) — never a `))`
//! hunt. The balanced-group-then-require-`)` verdict is command-position
//! only (parse.y:4900 `parse_dparen` -> parse.y:4970 `parse_arith_cmd`).
//! Inside double quotes the nested `$(` recursion (parse.y:4183-4188) keeps
//! the top-level `)` as quoted text and the closing `"` still delimits.
//!
//! Expectations below were verified byte-for-byte against WSL GNU Bash
//! 5.3.20 (probe scripts o1-o7/m3/wide in the task archive); the arith-true
//! pins guard the gate's positive side against drift.

use std::process::Command;

fn run(script: &str) -> (String, String, i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("rubash runs");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

#[test]
fn dquoted_assignment_subshell_or_list_is_one_word() {
    // The o1 shape: assignment RHS in double quotes with a trailing command
    // on the same line. Pre-fix the scanner swallowed `; echo after` into
    // the assignment word and printed nothing.
    let (stdout, stderr, code) = run("x=\"$(( echo hi ) || true)\"; echo \"B=[$x]\"; echo after");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "B=[hi]\nafter\n");
}

#[test]
fn function_body_dquoted_subshell_or_list_parses() {
    // The o7 shape (nvm's respawn-prelude face): the same construct inside a
    // function body. Pre-fix the swallow ate the closing `}` and the parse
    // died with "unexpected end of file from `{'".
    let (stdout, stderr, code) =
        run("f() { local A; A=\"$(( true | head -n 1 ) || true)\"; echo \"A=[$A]\"; }\nf");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "A=[]\n");
}

#[test]
fn midword_unquoted_subshell_or_list_stays_one_word() {
    // The word.rs skip_word_inner face: `$(( ... )` glued mid-word with text
    // after the close on the same line.
    let (stdout, stderr, code) = run("echo x$(( echo mid ) || true)y");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "xmidy\n");
}

#[test]
fn braced_default_subshell_or_list_closes_at_group() {
    // The ${} scan faces (skip_braced / run_skip_brace).
    let (stdout, stderr, code) = run("x=5; echo \"${x:-$(( echo d ) || true)}\"");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "5\n");
}

#[test]
fn nvm_resolve_alias_serialized_face() {
    // The exact construct nvm.sh:1336 becomes after the respawn-prelude
    // reprint (spaceless `$((`), inside a function, with pipelines, quoted
    // operands and a redirect — the minimal shape of the captured respawn
    // -c source. GNU: the substitution runs the subshell-or-list, `head -n
    // 1` passes `va` through, `A` gets `va`.
    let (stdout, stderr, code) = run(
        "nvm_alias() { echo va; }\nnvm_echo() { echo \"\"; }\n\
         f() { local A; A=\"$(( nvm_alias \"x\" 2> /dev/null | command head -n 1 | command tail -n 1 ) || nvm_echo)\"; echo \"f-got=[$A]\"; }\n\
         f",
    );
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "f-got=[va]\n");
}

#[test]
fn arith_true_forms_still_route_arith() {
    // Gate's positive side: real `$(( expr ))` arithmetic is untouched, in
    // and out of quotes, with nested parens and base-force (the
    // `$((10#${ARR[0]}))` shape from the #512 follow-up).
    let (stdout, stderr, code) = run(
        "echo $((1+2))\necho $(((1+2)*3))\necho \"$(( (1+2) * 3 ))\"\necho $((10#8))\nx=5; echo $((x*2))",
    );
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "3\n9\n9\n8\n10\n");
}

#[test]
fn arith_error_wording_unchanged() {
    // `$((echo one))` has a `))` pair and stays arithmetic; GNU reports the
    // arithmetic syntax error (parse_arith_cmd accepted the pair, the arith
    // evaluator rejected the words) and continues with the next command.
    let (stdout, stderr, _code) = run("echo $((echo one))\necho done");
    assert!(
        stderr.contains("arithmetic syntax error"),
        "stderr: {stderr}"
    );
    assert_eq!(stdout, "done\n");
}

#[test]
fn unclosed_arith_still_reports_eof_seeking_paren() {
    // Genuinely unterminated `$((` keeps the GNU diagnostics (verified
    // against 5.3.20): an open dquote wins (`x="$((1+2"` -> matching `"`,
    // exit 2); a bare `x=$((1+2` reports matching `)' at exit 2
    // (parse.y:3912's matched-pair EOF report).
    let (_stdout, stderr, code) = run("x=\"$((1+2\"");
    assert_eq!(code, 2, "stderr: {stderr}");
    assert!(
        stderr.contains("unexpected EOF while looking for matching `\"'"),
        "stderr: {stderr}"
    );
    let (_stdout, stderr, code) = run("x=$((1+2");
    assert_eq!(code, 2, "stderr: {stderr}");
    assert!(
        stderr.contains("unexpected EOF while looking for matching `)'"),
        "stderr: {stderr}"
    );
}
