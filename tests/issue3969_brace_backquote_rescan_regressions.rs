//! rubash#3969: GNU hands every brace product to word expansion as a
//! fresh word (braces.c:12750 brace_expand_word_list ->
//! subst.c:13062 shell_expand_word_list). A `{a..A}` sequence produces
//! the ASCII backquote (96) between `A`(65) and `a`(97), and when that
//! product is glued to further text the expansion scan's `` ` `` case
//! (subst.c:11568-11593) string_extracts to a closing backquote that
//! does not exist — `bad substitution: no closing "`" in …`, an
//! expand_word_error that abandons the whole word list
//! (subst.c:13093-13110). Only an isolated product ending in the
//! backquote stays literal (the bare-backquote compatibility exception,
//! subst.c:11574-11576, the #3968 behavior).
//!
//! Oracle: GNU bash 5.3.20 on WSL /usr/local/bin/bash, LF script files;
//! every expectation below was byte-compared there.

use std::process::Command;

fn run_c(script: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash -c probe")
}

fn stdout_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The command/word list the erroring product belongs to is discarded
/// (GNU exp_jump_to_top_level(DISCARD)), so `;`-joined text never runs;
/// the diagnostic carries the product's tail from the backquote.
fn assert_bad_substitution(script: &str, tail: &str) {
    let output = run_c(script);
    assert_eq!(stdout_of(&output), "", "stdout for {script:?}");
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains(&format!("bad substitution: no closing \"`\" in {tail}\n")),
        "stderr for {script:?}: {stderr:?}"
    );
    assert_eq!(
        stderr.matches("bad substitution").count(),
        1,
        "expected exactly one diagnostic for {script:?}: {stderr:?}"
    );
}

#[test]
fn isolated_backquote_product_stays_literal() {
    // #3968 contract: `{a..A}` alone is 33 words; index 2 (descending
    // from 'a') is the literal backquote, and the trailing bare `` ` ``
    // product is not an unclosed substitution.
    let output = run_c("set -- {a..A}; echo \"n=$# second=<$2>\"");
    assert_eq!(stdout_of(&output), "n=33 second=<`>\n");
    assert_eq!(stderr_of(&output), "");
    let output = run_c("a=({a..A}); echo \"n=${#a[@]} e1=<${a[1]}>\"");
    assert_eq!(stdout_of(&output), "n=33 e1=<`>\n");
    assert_eq!(stderr_of(&output), "");
}

#[test]
fn backquote_product_glued_to_parameter_fails() {
    assert_bad_substitution("set -- {a..A}$x", "`$x");
    assert_bad_substitution("set -- {a..A}${x}", "`${x}");
}

#[test]
fn backquote_product_glued_to_literal_fails() {
    assert_bad_substitution("set -- {a..A}x", "`x");
    assert_bad_substitution("set -- {a..A}xy", "`xy");
    // The single-quoted tail is word text the scan must read raw: the
    // `'` inside the extraction is ordinary data (string_extract,
    // subst.c:791).
    assert_bad_substitution("set -- {a..A}'x'", "`'x'");
    // An escaped backquote in the postamble cannot close the generated
    // one either: `\x` is one escaped character for the scan.
    assert_bad_substitution("set -- {a..A}\\`", "`\\`");
}

#[test]
fn backquote_product_glued_to_second_brace_fails() {
    assert_bad_substitution("set -- {a..A}{1..2}", "`1");
    assert_bad_substitution("set -- {a..A}{a..A}", "`a");
    // The tail starts at the backquote (string+t_index), not the product.
    assert_bad_substitution("set -- {1..2}{a..A}x", "`x");
}

#[test]
fn backquote_product_glued_to_comsub_or_dquote_fails() {
    assert_bad_substitution("set -- {a..A}$(echo hi)", "`$(echo hi)");
    assert_bad_substitution("set -- {a..A}\"x\"", "`\"x\"");
}

#[test]
fn products_before_the_erroring_one_are_not_emitted() {
    // GNU expands products in order and DISCARDs the whole list: the
    // `a`/`b` words before the backquote element never reach `set --`.
    assert_bad_substitution("set -- {A..z}x; echo n=$#", "`x");
    let output = run_c("set -- {a..A}x; echo n=$#\necho next");
    // The same `;`-joined echo is discarded with the word list; the next
    // command line still runs.
    assert_eq!(stdout_of(&output), "next\n");
}

#[test]
fn compound_array_assignment_fails_whole() {
    // The pre-fix bug paired the generated backquote ACROSS products and
    // executed the middle text (`1: command not found`); GNU abandons
    // the whole assignment instead.
    assert_bad_substitution("a=({a..A}{1..2}); echo n=${#a[@]}", "`1");
    assert_bad_substitution("a=({a..A}x); echo n=${#a[@]}", "`x");
    // declare-family compound args: the parens are token delimiters, so
    // the diagnostic tail ends at the body product, `)` excluded.
    assert_bad_substitution("declare -a h=({a..A}x); echo n=${#h[@]}", "`x");
    assert_bad_substitution("declare -a i=(p{a..A}q); echo n=${#i[@]}", "`q");
}

#[test]
fn for_select_compgen_and_redirect_lists_fail() {
    assert_bad_substitution("for w in {a..A}x; do echo $w; done; echo s=$?", "`x");
    assert_bad_substitution("compgen -W '{a..A}x' >/dev/null; echo s=$?", "`x");
    assert_bad_substitution("echo hi > {a..A}y; echo s=$?", "`y");
}

#[test]
fn balanced_backquote_pairs_in_products_still_execute() {
    // A generated backquote that CLOSES is a real command substitution;
    // the verdict only covers the unclosed case.
    let output = run_c("set -- {a..c}`echo hi`; echo \"n=$#\"; printf '%s\\n' \"$@\"");
    assert_eq!(stdout_of(&output), "n=3\nahi\nbhi\nchi\n");
    assert_eq!(stderr_of(&output), "");
    // Backquotes inside a double-quoted postamble stay live and close.
    let output = run_c("set -- {a..c}\"x`echo Q`y\"; echo \"n=$#\"; printf '%s\\n' \"$@\"");
    assert_eq!(stdout_of(&output), "n=3\naxQy\nbxQy\ncxQy\n");
    assert_eq!(stderr_of(&output), "");
}

#[test]
fn words_without_backquote_products_are_unaffected() {
    let output = run_c("set -- {a..c}x; echo \"n=$# $*\"");
    assert_eq!(stdout_of(&output), "n=3 ax bx cx\n");
    assert_eq!(stderr_of(&output), "");
    let output = run_c("x=XX; set -- {a..c}$x; echo \"n=$# $*\"");
    assert_eq!(stdout_of(&output), "n=3 aXX bXX cXX\n");
    assert_eq!(stderr_of(&output), "");
    // Prefix-glued products keep the bare-trailing-backquote exception:
    // `x{a..A}` ends each product in the backquote — literal data.
    let output = run_c("set -- x{a..A}; echo \"n=$# second=<$2>\"");
    assert_eq!(stdout_of(&output), "n=33 second=<x`>\n");
    assert_eq!(stderr_of(&output), "");
}
