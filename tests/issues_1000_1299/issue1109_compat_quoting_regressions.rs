//! rubash#1109 — compat quoting behaviors:
//! 1. patsub replacement quote removal flips off at compat <= 42
//!    (GNU subst.c: the replacement word is not quote-processed in
//!    4.2-and-earlier compatibility);
//! 2. POSIX mode: a single quote inside `${...}` in double quotes is
//!    literal data above compat 42 (Austin Group Interp 221), so the
//!    first unquoted `}` closes the expansion; at <= 42 it quotes again.
//!
//! All expectations byte-verified against GNU bash 5.3.20 (script files).

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
fn patsub_replacement_quotes_are_literal_at_compat42() {
    let (stdout, _, code) =
        rubash("shopt -s compat42; v=axc; echo \"${v/x/\"*\"}\" \"${v/x/'Q Q'}\"");
    assert_eq!(stdout, "a\"*\"c a'Q Q'c\n");
    assert_eq!(code, Some(0));
}

#[test]
fn patsub_replacement_quotes_removed_at_compat43_and_default() {
    for setup in ["shopt -s compat43; ", ""] {
        let (stdout, _, code) = rubash(&format!("{setup}v=axc; echo \"${{v/x/\"*\"}}\""));
        assert_eq!(stdout, "a*c\n", "setup {setup}");
        assert_eq!(code, Some(0), "setup {setup}");
    }
}

#[test]
fn patsub_single_quoted_dollar_still_expands_at_compat42() {
    // Only quote REMOVAL is off at <= 42: expansions inside the
    // replacement still run everywhere, even inside single quotes.
    let (stdout, _, code) = rubash("shopt -s compat42; y=Y; v=axc; echo \"${v/x/'$y'}\"");
    assert_eq!(stdout, "a'Y'c\n");
    assert_eq!(code, Some(0));
}

#[test]
fn posix_single_quote_in_braces_is_literal_above_compat42() {
    // Newline-separated (the filed repro's shape): `set -o posix` must
    // take effect before the echo line is tokenized.
    let (stdout, _, code) = rubash("set -o posix\nw=UNSET; echo \"C=${w:-a'}'b}\"");
    assert_eq!(stdout, "C=UNSET'b}\n");
    assert_eq!(code, Some(0));
}

#[test]
fn posix_single_quote_in_braces_quotes_at_compat42() {
    let (stdout, _, code) =
        rubash("shopt -s compat42\nset -o posix\nw=UNSET; echo \"C=${w:-a'}'b}\"");
    assert_eq!(stdout, "C=UNSET\n");
    assert_eq!(code, Some(0));
}

#[test]
fn posix_single_quote_rule_needs_posix_and_double_quotes() {
    // Non-POSIX: the quote pairs and the tail is part of the body.
    let (stdout, _, code) = rubash("w=UNSET; echo \"C=${w:-a'}'b}\"");
    assert_eq!(stdout, "C=UNSET\n");
    assert_eq!(code, Some(0));
    // POSIX but unquoted: the quote still quotes.
    let (stdout, _, code) = rubash("set -o posix; w=UNSET; echo C=${w:-a'}'b}");
    assert_eq!(stdout, "C=UNSET\n");
    assert_eq!(code, Some(0));
}
