//! Issue rubash#461: a `=~` regex word containing an UNQUOTED `|` followed
//! by a compound-command reserved word (`case`, `if`) was rejected inside a
//! `while`/`until` CONDITION — `while [[ x =~ x|case ]]; do` died with
//! `syntax error near unexpected token 'do'`. Residue of the #322 fix: the
//! parenthesized form and the top-level/`if` positions parsed, but the
//! loop-condition boundary scanner saw the lexer's Keyword `case` fragment
//! (reserved words ARE acceptable after `|`) and pushed a phantom `case`
//! frame whose `esac` never came, so `do` was never found at depth 0.
//!
//! GNU semantics (vendored third_party/bash): the whole `=~` RHS is ONE
//! word — parse.y:5443-5461 (read_token_word under PST_REGEXP) folds `|`
//! via got_character and `(...)` via parse_matched_pair, and parse.y:3663
//! `if (parser_state & PST_REGEXP) goto tokword` means reserved-word
//! recognition NEVER happens inside the regex word — in every conditional
//! position (while/until condition, if condition, top level, quoted or not),
//! and never outside `[[ ]]`.
//!
//! Fix: src/lexer/scanner.rs flags every fragment of the still-open `=~`
//! RHS word (Token::regexp_rhs_fragment); src/parser/support.rs makes
//! reserved-word recognition (is_keyword) and the compound-boundary stack
//! inert for flagged tokens.
//!
//! Oracle: Git for Windows GNU Bash 5.2.37 (D:\Git\usr\bin\bash.exe,
//! `bash -n` rc=0 on every script below, 2026-10-09); the issue records
//! WSL GNU Bash 5.3.0 rc=0 for the same shapes.

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

fn assert_clean(stderr: &str) {
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
}

/// The issue's minimal repro (as a run, with a bounded body): parses and
/// executes the loop body exactly once.
#[test]
fn while_condition_pipe_case_reserved_word() {
    let (stdout, stderr, code) = rubash("while [[ x =~ x|case ]]; do\n  echo hi\n  break\ndone");
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "hi\n");
}

/// until condition, `if` after the regex pipe.
#[test]
fn until_condition_pipe_if_reserved_word() {
    let (stdout, stderr, code) =
        rubash("until [[ x =~ x|if ]]; do\n  echo ran\n  break\ndone\necho ok");
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "ok\n");
}

/// Grouped regex with a reserved word after the pipe (issue table row).
#[test]
fn while_condition_grouped_pipe_case() {
    let (stdout, stderr, code) = rubash("while [[ a =~ (a|case) ]]; do\n  echo hi\n  break\ndone");
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "hi\n");
}

/// Variable LHS, the shellapi odsel shape (issue evidence, line 59 form).
#[test]
fn while_condition_odsel_shape() {
    let (stdout, stderr, code) = rubash(
        "y=\" case x\"\n\
         while [[ $y =~ ^[[:space:]]*(\\||case)[[:space:]]+(.*) ]]; do\n\
         echo hit:${BASH_REMATCH[2]}\n\
         break\n\
         done",
    );
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "hit:x\n");
}

/// `if` condition keeps working (issue table: already ok) and picks the
/// right branch.
#[test]
fn if_condition_pipe_case_still_ok() {
    let (stdout, stderr, code) =
        rubash("if [[ x =~ x|case ]]; then\n  echo then\nelse\n  echo else\nfi");
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "then\n");
}

/// Quoted forms were never affected: each reserved word stays literal data.
#[test]
fn quoted_regex_reserved_words_all_positions() {
    for keyword in ["case", "if", "while", "do", "done", "esac"] {
        // A quoted RHS is a LITERAL string in GNU (`[[ x =~ "a|b" ]]`
        // compares the text), so match the variable's own text.
        let script = format!(
            "v='case|{kw}'\nwhile [[ $v =~ \"$v\" ]]; do echo w; break; done\nif [[ $v =~ \"$v\" ]]; then echo i; fi\ncase x in\n  *) [[ $v =~ \"$v\" ]] && echo c ;;\nesac\n",
            kw = keyword
        );
        let (stdout, stderr, code) = rubash(&script);
        assert_clean(&stderr);
        assert_eq!(code, Some(0), "keyword {keyword}: script failed");
        assert_eq!(stdout, "w\ni\nc\n", "keyword {keyword}");
    }
}

/// Unquoted reserved words after the regex pipe stay DATA in every
/// conditional position: while, until, if, and a top-level `[[ ]]`.
#[test]
fn unquoted_regex_reserved_words_all_positions() {
    for keyword in [
        "case", "if", "while", "until", "do", "done", "esac", "fi", "then",
    ] {
        // `[[ zz =~ x|KW ]]` never matches, so the until body runs once.
        let script = format!(
            "while [[ x =~ x|{kw} ]]; do echo w; break; done\nuntil [[ zz =~ x|{kw} ]]; do echo u; break; done\nif [[ x =~ x|{kw} ]]; then echo i; fi\n[[ x =~ x|{kw} ]] && echo t\n",
            kw = keyword
        );
        let (stdout, stderr, code) = rubash(&script);
        assert_clean(&stderr);
        assert_eq!(code, Some(0), "keyword {keyword}: script failed");
        assert_eq!(stdout, "w\nu\ni\nt\n", "keyword {keyword}");
    }
}

/// The regex word must keep matching as a REGEX: `x|case` alternation
/// really matches `case` too, and `[[ case =~ x|case ]]` is true.
#[test]
fn regex_alternation_semantics_preserved() {
    let (stdout, stderr, code) =
        rubash("[[ case =~ x|case ]] && echo alt\n[[ zebra =~ x|case ]] || echo noalt");
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "alt\nnoalt\n");
}

/// Outside `[[ ]]` nothing changes: a real pipeline into a case command
/// still parses as shell grammar.
#[test]
fn real_pipeline_case_outside_conditional() {
    let (stdout, stderr, code) = rubash("echo case | while read w; do echo got:$w; break; done");
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "got:case\n");
}

/// The `+` metacharacter (issue title) and other regex specials after a
/// reserved word: still one word, still a regex.
#[test]
fn regex_metachars_after_reserved_word() {
    let (stdout, stderr, code) = rubash("x=accc\nwhile [[ $x =~ case+b ]]; do echo no; break; done\nwhile [[ $x =~ a+c ]]; do echo plus; break; done");
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "plus\n");
}

/// Nested loop bodies: the boundary scanner must not leak the fragment
/// flag past the enclosing constructs.
#[test]
fn regex_pipe_case_inside_nested_loop_body() {
    let (stdout, stderr, code) = rubash(
        "for i in 1 2; do\n  while [[ x =~ x|case ]]; do\n    echo i:$i\n    break\n  done\ndone",
    );
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "i:1\ni:2\n");
}

/// Function body scan (matching_function_loop_end walks raw tokens too).
#[test]
fn regex_pipe_case_inside_function_loop() {
    let (stdout, stderr, code) =
        rubash("f() {\n  while [[ x =~ x|case ]]; do\n    echo in f\n    break\n  done\n}\nf");
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "in f\n");
}
