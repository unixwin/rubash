//! rubash#3557 — quoted compound-marker head words are plain command
//! names, not shell syntax.
//!
//! GNU basis: `(`/`{`/`[[`/`((` and the reserved words `!`, `function`,
//! `case`, `for`, `select`, `coproc` are recognized only when they arrive
//! UNQUOTED in the token stream (parse.y:3667 read_token: `(` is an
//! operator only unquoted; `{`/`}`/reserved words likewise). A quoted
//! `'('`/`'{'`/`'[['`/`'(('` word is data: GNU runs it as a command name
//! and prints `<marker>: command not found` with status 127, then
//! continues.
//!
//! Rubash's alias re-lex probes (`execute_alias_introduced_*`) exist only
//! because GNU re-lexes alias VALUES through push_string
//! (parse.y:3249 alias_expand_token); a literal head word the parser
//! already froze as data may not take that path. Same for the parser's
//! `{`/`[[`/`((` value-based gates and the `LateSlot::Arithmetic`
//! words-only `((` claim.
//!
//! Expectations verified against GNU bash (`command not found`, rc 127,
//! script continues).

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

#[test]
fn quoted_subshell_marker_is_command_name() {
    // GNU: `(: command not found`, status 127, `AFTER` still runs.
    let (stdout, stderr, code) = rubash("'(' echo hi ')'; echo post=$?; echo AFTER");
    assert!(stderr.contains("(: command not found"), "stderr: {stderr}");
    assert!(stdout.contains("post=127\nAFTER"), "stdout: {stdout}");
    assert_eq!(code, Some(0));
}

#[test]
fn quoted_brace_marker_is_command_name() {
    // GNU: `{: command not found` — NOT "syntax error near unexpected
    // end of file".
    let (stdout, stderr, code) = rubash("'{' echo hi '}'; echo post=$?; echo AFTER");
    assert!(stderr.contains("{: command not found"), "stderr: {stderr}");
    assert!(
        !stderr.contains("unexpected end of file"),
        "stderr: {stderr}"
    );
    assert!(stdout.contains("post=127\nAFTER"), "stdout: {stdout}");
    assert_eq!(code, Some(0));
}

#[test]
fn quoted_conditional_and_arithmetic_markers_are_command_names() {
    let (_, stderr, _) = rubash("'[[' x = x ']]'");
    assert!(stderr.contains("[[: command not found"), "stderr: {stderr}");
    let (_, stderr, _) = rubash("'((' 1 '))'");
    assert!(stderr.contains("((: command not found"), "stderr: {stderr}");
}

#[test]
fn quoted_reserved_word_heads_are_command_names() {
    // `!`, `function`, `case`, `for`, `select`, `coproc` quoted: GNU
    // reports `<word>: command not found` (127) — no inversion, no
    // compound reparse.
    for marker in ["!", "function", "case", "for", "select", "coproc"] {
        let script = format!("'{marker}' x; echo post=$?");
        let (stdout, stderr, _) = rubash(&script);
        assert!(
            stderr.contains(&format!("{marker}: command not found")),
            "marker {marker} stderr: {stderr}"
        );
        assert!(
            stdout.contains("post=127"),
            "marker {marker} stdout: {stdout}"
        );
    }
}

#[test]
fn unquoted_markers_still_form_compounds() {
    // Regression guard the other way: the unquoted forms must keep
    // working — the fix narrows admission, it does not remove the paths.
    let (stdout, _, _) = rubash("( echo SUB); { echo BR; }; ((x=5)); echo x=$x; [[ 1 = 1 ]] && echo COND; ! true; echo inv=$?");
    assert!(stdout.contains("SUB"), "stdout: {stdout}");
    assert!(stdout.contains("BR"), "stdout: {stdout}");
    assert!(stdout.contains("x=5"), "stdout: {stdout}");
    assert!(stdout.contains("COND"), "stdout: {stdout}");
    assert!(stdout.contains("inv=1"), "stdout: {stdout}");
}

#[test]
fn alias_introduced_paren_still_relexes() {
    // The alias path the probes emulate: `alias lp='('` expands to a
    // real `(` operator under shopt -s expand_aliases — GNU re-lexes
    // it as a subshell. Keep that working.
    let (stdout, _, _) = rubash("shopt -s expand_aliases\nalias lp='('\nlp echo ALIAS_HI )");
    assert!(stdout.contains("ALIAS_HI"), "stdout: {stdout}");
}
