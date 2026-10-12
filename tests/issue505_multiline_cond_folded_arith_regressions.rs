//! Regressions for unixwin/rubash#505: a multi-line `[[ ]]` expression whose
//! continuation line starts with `||` (or holds the closing `]]`) and whose
//! operand is a folded `((...))` arithmetic group was rejected inside a
//! function body ("unexpected token `newline', conditional binary operator
//! expected") while GNU bash accepts it.
//!
//! GNU anchor: read_token delivers `(' as the grouping token inside a
//! conditional — the arithmetic-command fold is gated on
//! `(parser_state & (PST_CASEPAT|PST_CONDCMD)) == 0' (parse.y:3780) — so
//! `[[ ((x == 0)) ]]' parses as cond_term group(group(binary))
//! (parse.y:5094-5110). Every completed term ends with
//! `(void)cond_skip_newlines ()' (parse.y:5108 group close, parse.y:5140
//! unary, parse.y:5243 binary rhs), which is why a newline before a
//! following operand, `||', or `]]' is invisible after a CLOSED GROUP but
//! is a hard error in a bare word's binop slot (raw read_token,
//! parse.y:5179: `[[ x\n]]' → "conditional binary operator expected").
//! rubash's lexer folds a balanced `((...))' at command position into one
//! Keyword token regardless of the conditional; the cond collector then
//! modeled it as a single WORD operand whose binop slot surfaced the
//! continuation-line newline. The fix re-splits the fold into the group
//! tokens GNU's reader would have produced.
//!
//! Real-world evidence: sheldritch/sheldritch core/args.sh:1303-1308
//! (56 KB, harvest 2026-10-12 — the sole SYNTAX-REJECT-RUBASH-ONLY asset
//! in the wt92 eco re-audit still failing on master).
//!
//! Every accept/reject case below was pinned against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, the repo baseline) via script files.

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

fn run_rubash_capture(script: &str) -> (String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
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

/// GNU rejects; rubash must keep rejecting with the same diagnostic.
fn assert_rejects(script: &str, expected_fragment: &str) {
    let (_, stderr, code) = run_rubash(script);
    assert_eq!(code, Some(2), "expected syntax error for {script:?}");
    assert!(
        stderr.contains(expected_fragment),
        "expected {expected_fragment:?} in stderr {stderr:?}"
    );
}

// ---------------------------------------------------------------------------
// The issue's minimal repro and its element matrix
// ---------------------------------------------------------------------------

#[test]
fn issue_repro_function_body_multiline_or_continuation() {
    // GNU bash 5.3.0: bash -n rc=0, run prints IN-FUNC-OK.
    assert_parses(
        "f() {\nif [[\n  ( -z \"$ExactMatch\" && \"$ArityMin\" = \"$ArityMax\" )\n  || ((${#_ARGS_BOUNDS[@]} == 0))\n]]; then\n  :\nfi\n}\n",
    );
}

#[test]
fn issue_repro_runs_like_gnu() {
    let (stdout, code) = run_rubash_capture(
        "f() {\nif [[\n  ( -z \"$ExactMatch\" && \"$ArityMin\" = \"$ArityMax\" )\n  || ((${#_ARGS_BOUNDS[@]} == 0))\n]]; then\n  echo IN-FUNC-OK\nfi\n}\nExactMatch=\"\"\nArityMin=1\nArityMax=1\nf\n",
    );
    assert_eq!(stdout, "IN-FUNC-OK\n");
    assert_eq!(code, Some(0));
}

#[test]
fn args_sh_1307_shape_tab_indented_in_function() {
    // The asset's exact shape: tab indent, comment lines inside the
    // expression, `|| ((${#...} == 0))` continuation.
    assert_parses(
        "f() {\n\t\tif [[\n\t\t\t# prioritise exact argument matches\n\t\t\t( -z \"$ExactMatch\" && \"$ArityMin\" = \"$ArityMax\" )\n\t\t\t# prioritise existing bounds\n\t\t\t|| ((${#_ARGS_BOUNDS[@]} == 0))\n\t\t]]; then\n\t\t\techo ok\n\t\tfi\n}\nf\n",
    );
}

#[test]
fn function_if_multiline_group_or_plain_group() {
    assert_parses("f() {\nif [[\n  ( -z \"$x\" )\n  || (1 == 1)\n]]; then echo ok; fi\n}\n");
}

// ---------------------------------------------------------------------------
// Folded ((...)) as the continuation-line operand
// ---------------------------------------------------------------------------

#[test]
fn function_if_multiline_folded_arith_alone() {
    // GNU: group(group(binary)); the group close's cond_skip_newlines
    // hides the newline before `]]` (pinned: PASS).
    assert_parses("f() {\nif [[\n  ((${#A[@]} == 0))\n]]; then echo ok; fi\n}\n");
}

#[test]
fn function_while_multiline_folded_arith_continuation() {
    // while-condition face of the same bug (also fails via f() ( ... )).
    assert_parses(
        "f() {\nwhile [[\n  ( -z \"$x\" )\n  || ((${#A[@]} == 0))\n]]; do break; done\n}\n",
    );
}

#[test]
fn function_paren_body_multiline_folded_arith_continuation() {
    assert_parses(
        "f() (\nif [[\n  ( -z \"$x\" )\n  || ((${#A[@]} == 0))\n]]; then echo ok; fi\n)\n",
    );
}

#[test]
fn folded_arith_close_newline_then_bracket_bracket() {
    // GNU p7: `[[ ((x))\n]]' — group close swallows the newline (PASS).
    assert_parses("f() {\n[[ ((x))\n]] && echo ok\n}\nf\n");
}

#[test]
fn folded_arith_multiline_chain() {
    // GNU p10: every operand is a closed group; newlines between them are
    // invisible (PASS).
    assert_parses("[[ ((x))\n&& ((y))\n]] && echo ok\n");
}

#[test]
fn folded_arith_single_line_controls_stay_accepted() {
    // GNU p3/p4/p9 controls: no behavior change for the one-line forms.
    assert_parses("[[ ((x)) ]] && echo ok\n");
    assert_parses("x=; if [[ (( ${#x} == 0 )) ]]; then echo ok; fi\n");
    assert_parses("[[ ((x)) && ((y)) ]] && echo ok\n");
    assert_parses("[[ ( (x) ) ]] && echo ok\n");
}

// ---------------------------------------------------------------------------
// GNU-reject controls: the fix must not loosen these
// ---------------------------------------------------------------------------

#[test]
fn bare_word_operand_newline_still_rejected() {
    // GNU p12/g1: a bare word's binop slot is a raw read_token — the
    // newline IS the error there.
    assert_rejects(
        "f() {\nwhile [[ x\n]]; do break; done\n}\n",
        "conditional binary operator expected",
    );
}

#[test]
fn group_then_binop_still_rejected() {
    // GNU q4: the group's trailing cond_skip_newlines leaves `==` in
    // cond_token; parse_cond_command's COND_END check rejects (standalone
    // form — inside an and_or/if the pre-existing marker-surfacing gap
    // hides the node; tracked with the #551 diagnostics family).
    assert_rejects(
        "[[ (x) == x ]]\n",
        "syntax error in conditional expression: unexpected token `=='",
    );
}

#[test]
fn folded_group_then_binop_still_rejected() {
    // GNU q5.
    assert_rejects(
        "[[ ((x)) == x ]]\n",
        "syntax error in conditional expression: unexpected token `=='",
    );
}

#[test]
fn folded_group_after_oror_then_binop_still_rejected() {
    // GNU q2: the lexer-folded `((x))` after `||` must behave like the
    // unfolded group form — GNU rejects with the cond_error token message.
    assert_rejects(
        "[[ ( -z x ) || ((x)) == y ]]\n",
        "syntax error in conditional expression: unexpected token `=='",
    );
}

#[test]
fn folded_group_after_oror_then_junk_word_still_rejected() {
    // GNU q1/q6: junk after the closed group lands in cond_token.
    assert_rejects(
        "[[ ( -z x ) || ((x)) junk ]]\n",
        "syntax error in conditional expression: unexpected token `junk'",
    );
}
