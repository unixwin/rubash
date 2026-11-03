//! Case-pattern acceptance (GNU parse.y:1225-1236 pattern_list: every
//! alternative is ONE word; `|' only separates complete alternatives;
//! unquoted redirection operators are not pattern text):
//! - rubash#2678 (A1-F04): after a `;&' fall-through terminator the
//!   following tokens are still clause-pattern context — a second word
//!   in pattern position is a syntax error at that word, not a new
//!   command and not more pattern text.
//! - rubash#2687 (A1-F17): unquoted `<`/`>` (and the other redirect
//!   operators) in pattern position are operators, not literals —
//!   GNU reports them; rubash used to drop them from the pattern.
//!   A `<(...)'/`>(...)' process substitution IS a pattern word.
//! - rubash#2744 (B6-R6): a pattern list may not start (or end, or
//!   double) its `|' separators.
use std::{env, process::Command};

fn run(source: &str) -> (String, String, i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(source)
        .output()
        .expect("run rubash -c");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

fn assert_syntax_error_near(source: &str, token: &str) {
    let (stdout, stderr, code) = run(source);
    assert_eq!(code, 2, "source: {source}\nstdout: {stdout}\nstderr: {stderr}");
    assert_eq!(stdout, "", "source: {source}");
    assert!(
        stderr.contains(&format!("syntax error near unexpected token `{token}'")),
        "source: {source}\nexpected blame `{token}', stderr: {stderr}"
    );
}

#[test]
fn issue2744_pattern_list_may_not_start_with_a_pipe() {
    assert_syntax_error_near("case x in |a) echo hit;; esac", "|");
}

#[test]
fn issue2744_double_pipe_is_the_or_operator_error() {
    assert_syntax_error_near("case a in a||b) echo hit;; esac", "||");
}

#[test]
fn issue2744_trailing_pipe_blames_the_close_paren() {
    assert_syntax_error_near("case a in a|) echo hit;; esac", ")");
}

#[test]
fn issue2744_empty_clause_blames_the_close_paren() {
    assert_syntax_error_near("case x in ) echo hit;; esac", ")");
}

#[test]
fn issue2744_and_operator_in_pattern_position_errors() {
    assert_syntax_error_near("case a in a&&b) echo hit;; *) echo miss;; esac", "&&");
}

#[test]
fn issue2678_second_word_after_fallthrough_terminator_errors() {
    assert_syntax_error_near(
        "case x in x) echo one;& case y in y) echo FALL;; esac",
        "y",
    );
}

#[test]
fn issue2678_second_word_in_pattern_errors() {
    assert_syntax_error_near("case ab in a b) echo hit;; *) echo miss;; esac", "b");
    assert_syntax_error_near("case ab in a if) echo hit;; *) echo miss;; esac", "if");
    assert_syntax_error_near("case bc in a|b c) echo hit;; *) echo miss;; esac", "c");
}

#[test]
fn issue2678_valid_fallthrough_still_works() {
    let (stdout, stderr, code) = run("case x in x) echo one ;& y) echo FALL;; esac");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "one\nFALL\n");

    let (stdout, stderr, code) = run("case x in x) echo one ;;& y) echo FALL;; esac");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "one\n");
}

#[test]
fn issue2687_unquoted_redirect_operators_in_patterns_error() {
    assert_syntax_error_near("case \"a<\" in a<*) echo hit;; *) echo miss;; esac", "<");
    assert_syntax_error_near("case \"a>\" in a>*) echo hit;; *) echo miss;; esac", ">");
    assert_syntax_error_near("case \"a\" in a>>) echo hit;; *) echo miss;; esac", ">>");
    assert_syntax_error_near("case \"a\" in a<<EOF) echo hit;; *) echo miss;; esac", "<<");
}

#[test]
fn issue2687_escaped_and_quoted_operators_stay_literal() {
    let (stdout, stderr, code) = run("case \"a<\" in a\\<*) echo hit;; *) echo miss;; esac");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "hit\n");

    let (stdout, stderr, code) = run("case \"a<\" in \"a<\"*) echo hit;; *) echo miss;; esac");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "hit\n");
}

#[test]
fn issue2687_process_substitution_is_a_pattern_word() {
    let (stdout, stderr, code) = run("case x in <(ls)) echo hit;; *) echo miss;; esac");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "miss\n");
}

#[test]
fn issue2678_multi_segment_single_words_still_match() {
    // Adjacent segments are one word: quote runs, expansions and
    // extglob groups must keep reassembling into a single pattern.
    let (stdout, _, code) = run("case abc in a\"b\"c) echo hit;; *) echo miss;; esac");
    assert_eq!(code, 0);
    assert_eq!(stdout, "hit\n");

    let (stdout, _, code) = run("x=b\ncase abc in a$x) echo hit;; *) echo miss;; esac");
    assert_eq!(code, 0);
    assert_eq!(stdout, "miss\n");

    let (stdout, _, code) = run("x=bc\ncase abc in a$x) echo hit;; *) echo miss;; esac");
    assert_eq!(code, 0);
    assert_eq!(stdout, "hit\n");

    let (stdout, _, code) =
        run("shopt -s extglob\ncase ab in @(a|b)b) echo hit;; *) echo miss;; esac");
    assert_eq!(code, 0);
    assert_eq!(stdout, "hit\n");

    let (stdout, _, code) = run("case a in a ) echo hit;; esac");
    assert_eq!(code, 0);
    assert_eq!(stdout, "hit\n");
}
