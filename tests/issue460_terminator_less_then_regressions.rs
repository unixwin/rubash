//! Regressions for unixwin/rubash#460: a terminator-less `then' directly
//! after a closed `[[ ]]` (or `(( ))`) was rejected inside a function body
//! while GNU bash accepts it.
//!
//! GNU grammar: `if_command: IF compound_list THEN ...' where
//! `compound_list: newline_list list0 | newline_list list1'
//! (parse.y:1262) and `list1: ... | pipeline_command' (parse.y:1323) —
//! a single pipeline command needs NO list terminator, and
//! `reserved_word_acceptable' (parse.y:5899, COND_END and ARITH_CMD in
//! the predecessor set) recognizes `then'/`do' right after `]]'/`))'.
//! rubash's command loop demanded a connector (`;', `&', newline, ...)
//! after any complete compound command, so the `then` fired the
//! "syntax error near unexpected token `then'" node inside the if/loop
//! condition section — visible only through the function-body strict
//! re-parse (find_body_parse_error), which is why `bash -n' failed only
//! with the function wrapper while the same `if' at top level passed.
//!
//! Real-world evidence: GaelGosse/shell_function_n_shortcuts/.shortcut
//! (battery-status prompt), harvest 2026-10-04.
//!
//! Every accept/reject case below was pinned against GNU bash
//! (`D:\Git\usr\bin\bash.exe`, GNU bash 5.2.37 msys — a 5.2/5.3 split is
//! not expected for this grammar production; noted in the PR).

use std::process::Command;

fn run_rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-n")
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

/// GNU accepts (rc 0 after `bash -n`).
fn assert_parses(script: &str) {
    let (_, stderr, code) = run_rubash(script);
    assert_eq!(
        code,
        Some(0),
        "expected clean parse for {script:?}, stderr={stderr:?}"
    );
}

/// GNU rejects (`if true then', `if echo hi >/dev/null then', `while [[ ]]
/// then'-with-`done' forms): rubash must keep rejecting too.
fn assert_rejects(script: &str) {
    let (_, _, code) = run_rubash(script);
    assert_eq!(code, Some(2), "expected syntax error for {script:?}");
}

// ---------------------------------------------------------------------------
// The issue's minimal repro and its body-context family
// ---------------------------------------------------------------------------

#[test]
fn issue_repro_function_body_cond_then() {
    assert_parses("f() {\n  if [[ $x == y ]] then\n    echo a\n  fi\n}\n");
}

#[test]
fn issue_repro_runs_when_defined_and_called() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("f() {\n  if [[ $1 == y ]] then\n    echo a\n  fi\n}\nf y\nf n\n")
        .output()
        .expect("run rubash");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "a\n");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn subshell_body_cond_then() {
    assert_parses("f() (\n  if [[ $x == y ]] then\n    echo a\n  fi\n)\n");
}

#[test]
fn same_line_function_body_cond_then() {
    assert_parses("f() { if [[ $x == y ]] then echo a; fi; }\n");
}

#[test]
fn elif_cond_then() {
    assert_parses(
        "f() {\n  if [[ $x == y ]] then\n    echo a\n  elif [[ $x == z ]] then\n    echo b\n  fi\n}\n",
    );
}

#[test]
fn nested_brace_group_cond_then() {
    assert_parses("g() {\n  {\n    if [[ $x == y ]] then\n      echo a\n    fi\n  }\n}\n");
}

// ---------------------------------------------------------------------------
// Terminator shapes after ]] — accept/reject both pinned to GNU
// ---------------------------------------------------------------------------

#[test]
fn semicolon_terminated_then_still_parses() {
    assert_parses("f() {\n  if [[ $x == y ]]; then\n    echo a\n  fi\n}\n");
}

#[test]
fn newline_terminated_then_still_parses() {
    assert_parses("f() {\n  if [[ $x == y ]]\n  then\n    echo a\n  fi\n}\n");
}

#[test]
fn non_conditional_then_still_rejected() {
    // After a plain WORD, `then' is an argument, not the reserved word.
    assert_rejects("f() {\n  if true then\n    echo a\n  fi\n}\n");
}

#[test]
fn redirect_then_still_rejected() {
    assert_rejects("if echo hi >/dev/null then\necho a\nfi\n");
}

#[test]
fn while_cond_done_after_cond_still_rejected() {
    // `while [[ ]] then ... done` mismatches THEN with DONE — GNU rejects.
    assert_rejects("f() {\n  while [[ $x == y ]] then\n    :\n  done\n}\n");
}

// ---------------------------------------------------------------------------
// Arithmetic (( )) predecessor — same reserved_word_acceptable family
// ---------------------------------------------------------------------------

#[test]
fn arith_cond_then_top_level() {
    assert_parses("if (( x == y )) then\necho a\nfi\n");
}

#[test]
fn arith_cond_then_function_body() {
    assert_parses("f() {\nif (( x == y )) then\necho a\nfi\n}\n");
}

#[test]
fn arith_cond_do_loop() {
    assert_parses("while (( x )) do\n:\ndone\n");
}

#[test]
fn cond_do_loop_function_body() {
    assert_parses("f() {\nwhile [[ x == y ]] do\n:\ndone\n}\n");
}

// ---------------------------------------------------------------------------
// `[[ ]]` existing semantics must not regress
// ---------------------------------------------------------------------------

#[test]
fn plain_conditional_command_unchanged() {
    assert_parses("[[ a == a ]] && echo ok\n[[ a == b ]] || echo no\n");
}

#[test]
fn cond_syntax_errors_unchanged() {
    assert_rejects("[[ a == ]]\n");
    assert_rejects("[[ -n\n");
}

#[test]
fn cond_as_last_command_of_function_unchanged() {
    assert_parses("f() {\n  [[ $1 == y ]]\n}\nf y\n");
}

#[test]
fn cond_followed_by_word_without_connector_still_rejected() {
    assert_rejects("[[ a == a ]] echo x\n");
}
