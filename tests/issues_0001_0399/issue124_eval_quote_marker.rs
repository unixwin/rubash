//! Issue unixwin/niubash#124 regression: `eval` must re-parse its argument the
//! way GNU does when the argument was built with POSIX single-quote escaping
//! (`'\''`).
//!
//! GNU expand_word_internal's single-quote arm (subst.c:11882) takes the
//! region body from string_extract_single_quoted (subst.c:1088), which only
//! substrings the raw text, and then calls remove_quoted_escapes
//! (subst.c:11902 -> 4901 -> dequote_escapes:4692), which strips only
//! CTLESC-CTLESC / CTLESC-CTLNUL pairs. A `"` written inside single quotes
//! therefore survives into the word value as a bare quote character, and since
//! eval passes `string_list` of its expanded arguments straight to
//! parse_and_execute (builtins/eval.def:49 -> evalstring), that quote delimits
//! again on the second parse.
//!
//! Rubash's lexer instead carries such a quote as the data-double-quote marker
//! (\x18) so the expansion walker does not treat it as a delimiter, and
//! eval_source_for_reparse has to render it back to source before the string is
//! parsed again. Without that restore, `eval 'd='\''abc'\''; echo "$d"'` handed
//! the literal quotes to the inner command, which is what broke Hermes-style
//! wrappers: their every command is `eval '<cmd with '\'' escapes>'`, so
//! `mkdir -p "$d"` received `'"abc"'` and the write tool failed.

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

/// The reporter's minimal shape, reduced to builtins so the test needs no
/// external commands: the double quotes in the eval string must be delimiters
/// for the inner parse, not literal characters.
#[test]
fn eval_single_quote_escape_concat_keeps_double_quotes_as_syntax() {
    let (stdout, stderr, code) = rubash(r#"eval 'd='\''abc'\''; echo "x y"'; echo "rc=$?""#);
    assert_eq!(stdout, "x y\nrc=0\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

/// A quoted `"$d"` expanded inside the eval string must still be one word.
#[test]
fn eval_quoted_variable_inside_concat_is_one_word() {
    let (stdout, _stderr, _code) =
        rubash(r#"eval 'd='\''a b'\''; set -- "pre-$d" tail; echo "$#[$1][$2]"'"#);
    assert_eq!(stdout, "2[pre-a b][tail]\n");
}

/// Builtin output keeps the value free of quote characters.
#[test]
fn eval_quoted_variable_printf_has_no_literal_quotes() {
    let (stdout, _stderr, _code) = rubash(r#"eval 'd='\''abc'\''; printf "[%s]" "$d"'; echo"#);
    assert_eq!(stdout, "[abc]\n");
}

/// The whole Hermes payload shape: assignment, mkdir-like redirect, quoted
/// argument, `$?` report — all inside one `eval` argument.
#[test]
fn eval_hermes_wrapper_payload_shape() {
    let (stdout, _stderr, _code) =
        rubash(r#"eval 'set -e; d='\''/tmp'\''; echo "d=[$d]"; printf "%s\n" "rc=$?"'; echo done"#);
    assert_eq!(stdout, "d=[/tmp]\nrc=0\ndone\n");
}

/// Control: a fully single-quoted eval argument (no `'\''` concatenation) was
/// never affected; keep it pinned so the fix does not regress the fast path.
#[test]
fn fully_single_quoted_eval_argument_still_delimits() {
    let (stdout, _stderr, _code) = rubash(r#"eval 'echo "x y"'; echo "rc=$?""#);
    assert_eq!(stdout, "x y\nrc=0\n");
}

/// A balanced literal `"` pair groups words the same way GNU does: the text is
/// re-read as source, so `eval echo \"a b\"` behaves like `eval 'echo "a b"'`.
#[test]
fn balanced_literal_double_quotes_group_the_inner_word() {
    let (stdout, _stderr, _code) = rubash(r#"eval echo \"a b\""#);
    assert_eq!(stdout, "a b\n");
}

/// The reparse text carries real delimiters, so a quote that closes a region
/// opened outside the eval string still pairs across the boundary:
/// `eval 'echo "a" "b"'` collapses to two arguments, not literal quote runs.
///
/// Note: an *unterminated* quote inside the eval string (`eval 'echo a"b'`) is
/// still accepted as data by eval's reparse while GNU reports an
/// unterminated-quote syntax error at top level; that is a separate lexer gap
/// and is deliberately not pinned here.
#[test]
fn delimiters_inside_eval_text_pair_across_the_boundary() {
    let (stdout, _stderr, code) = rubash(r#"eval 'echo "a" "b"'; echo "rc=$?""#);
    assert_eq!(stdout, "a b\nrc=0\n");
    assert_eq!(code, Some(0));
}
