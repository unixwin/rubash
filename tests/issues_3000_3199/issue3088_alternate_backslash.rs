//! rubash#3088 — `${var:-word}` / `${var-word}` / `${var+word}` alternate
//! results lose backslash protection: expansion-result backslashes are
//! folded out of the alternate word half.
//!
//! Root cause (two fold sites, one mechanism): GNU expands the alternate
//! word through the same single-pass word expansion as an ordinary word
//! (subst.c:4462 `expand_string_for_rhs` → subst.c:11229
//! `expand_word_internal`; subst.c:7966 `parameter_brace_expand_rhs`),
//! and `quote_string` (subst.c:4773) CTLESC-protects every expansion
//! result so the final dequote (subst.c:4807 `dequote_string`, which
//! strips only CTLESC bytes) can never read expansion data as source
//! escape syntax. Rubash re-unescapes the expanded alternate result:
//!
//! (A) Double-quoted context: `expand_operator_rhs_for_context`
//! (`src/executor/parameter_words.rs`) expands the alternate with the
//! protecting mut walker — which restores U+E30D backslashes to raw `\`
//! — and then runs `unescape_parameter_operator_result` →
//! `unescape_double_quoted_backslashes` over the RESULT, folding raw
//! `\$` / `` \` `` / `\"` / `\}` pairs that came from a variable's
//! VALUE (data) as if they were source double-quote escapes. The
//! distinguishing fold table proves it: `\$`, `` \` ``, `\"`, `\}` fold
//! while `\q`, `\ `, `\'`, `\*`, `\n`, `\\q` do not — exactly the
//! double-quote escape set. This site fires with and without a comsub
//! suffix, and for a comsub sitting OUTSIDE the `${}` as well, because
//! the fold happens inside the fragment evaluation itself.
//!
//! (B) Unquoted context / assignment RHS with a comsub suffix: the
//! alternate walker (`expand_embedded_parameters_alternate_mut`,
//! `protect_rhs_data=false`) splices `$y` results bare, and the
//! `$(`/backquote-gated trailing `unescape_remaining_shell_escapes`
//! pass in `expand_embedded_parameters_mut_inner` folds them — the
//! #2999 REPORT §6 residual 1. Nested alternates fold once per level
//! (inner restore + outer splice/trailing pass).
//!
//! The fix gives the alternate word half the same expansion-result
//! protection as the ordinary walker (protect expansion-result
//! backslashes as U+E30D through every post-expansion unescape pass,
//! restore at the outermost boundary) — never re-reading expansion
//! data as source escape syntax.
//!
//! Expectations below are GNU bash 5.3.20 byte output, verified against
//! the probe scripts under `~/workspace/fix-3088/probes/` (matrix
//! `run-matrix.sh`, base rubash `1c132f68`). Two transcription notes:
//! the issue's literal nested text (double-quoted, 2 backslashes) gives
//! GNU `['a\']`, NOT the issue's claimed `[a\\$b]` — the `'` chars are
//! literal data inside `"..."` and `$b` expands to empty — so the nested
//! pins use the #2999 REPORT form (unquoted, 1 backslash), and the
//! double-quoted literal forms are pinned as controls with their true
//! GNU expectations. `\\` inside a double-quoted alternate IS a live
//! escape in GNU (`['a\b']`), so i10 is a control, not a failure.
//!
//! Harness: `rubash -c` via `CARGO_BIN_EXE_rubash`, byte-level stdout
//! assertions. `LC_ALL=C.UTF-8` is set on the child explicitly so the
//! pin is hermetic under any ambient locale.

use std::process::{Command, Stdio};

fn run(script: &str) -> Vec<u8> {
    let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let output = child.wait_with_output().expect("wait rubash");
    assert!(
        output.status.success(),
        "rubash -c {script:?} exited with {}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn expect(script: &str, expected: &[u8], ctx: &str) {
    let out = run(script);
    assert_eq!(out, expected, "{ctx}: got {out:?}, want {expected:?}");
}

// --- Flat, double-quoted, no comsub suffix (site A) ---

#[test]
fn flat_dq_two_backslashes_preserved() {
    expect(
        r#"y='a\\$b'; x=; echo "[${x:-$y}]""#,
        b"[a\\\\$b]\n",
        "flat :- in double quotes must keep both backslashes of a\\\\$b value",
    );
}

#[test]
fn flat_dq_one_backslash_preserved() {
    expect(
        r#"y='a\$b'; x=; echo "[${x:-$y}]""#,
        b"[a\\$b]\n",
        "flat :- in double quotes must keep \\$ of an a\\$b value",
    );
}

#[test]
fn flat_dq_backtick_preserved() {
    expect(
        r#"y='a\`b'; x=; echo "[${x:-$y}]""#,
        b"[a\\`b]\n",
        "flat :- in double quotes must keep \\` of an a\\`b value",
    );
}

#[test]
fn flat_dq_dquote_preserved() {
    expect(
        r#"y='a\"b'; x=; echo "[${x:-$y}]""#,
        b"[a\\\"b]\n",
        "flat :- in double quotes must keep \\\" of an a\\\"b value",
    );
}

#[test]
fn flat_dq_rbrace_preserved() {
    expect(
        r#"y='a\}b'; x=; echo "[${x:-$y}]""#,
        b"[a\\}b]\n",
        "flat :- in double quotes must keep \\} of an a\\}b value",
    );
}

#[test]
fn flat_dq_no_backslash_control() {
    expect(
        r#"y='ab'; x=; echo "[${x:-$y$(printf '')}]""#,
        b"[ab]\n",
        "alternate without backslashes must stay unchanged",
    );
}

#[test]
fn flat_dq_backslash_b_control() {
    expect(
        r#"y='a\\b'; x=; echo "[${x:-$y}]""#,
        b"[a\\\\b]\n",
        "\\\\b (no escapable char) was already correct and must stay so",
    );
}

#[test]
fn flat_dq_backslash_q_control() {
    expect(
        r#"y='a\qb'; x=; echo "[${x:-$y}]""#,
        b"[a\\qb]\n",
        "\\q was already correct and must stay so",
    );
}

// --- Flat, double-quoted, comsub / backquote suffix ---

#[test]
fn flat_dq_comsub_one_backslash_preserved() {
    expect(
        r#"y='a\$b'; x=; echo "[${x:-$y$(printf '')}]""#,
        b"[a\\$b]\n",
        "flat :- with empty comsub suffix must keep \\$",
    );
}

#[test]
fn flat_dq_comsub_two_backslashes_preserved() {
    expect(
        r#"y='a\\$b'; x=; echo "[${x:-$y$(printf '')}]""#,
        b"[a\\\\$b]\n",
        "flat :- with empty comsub suffix must keep both backslashes",
    );
}

#[test]
fn flat_dq_backquote_suffix_preserved() {
    expect(
        r#"y='a\\$b'; x=; echo "[${x:-$y`printf ''`}]""#,
        b"[a\\\\$b]\n",
        "flat :- with backquote suffix must keep both backslashes",
    );
}

// --- Flat, unquoted ---

#[test]
fn flat_unquoted_one_backslash_preserved() {
    expect(
        r#"y='a\$b'; x=; echo [${x:-$y}]"#,
        b"[a\\$b]\n",
        "flat :- unquoted must keep \\$",
    );
}

#[test]
fn flat_unquoted_two_backslashes_preserved() {
    expect(
        r#"y='a\\$b'; x=; echo [${x:-$y}]"#,
        b"[a\\\\$b]\n",
        "flat :- unquoted must keep both backslashes",
    );
}

// --- Nested alternates (#2999 REPORT form: unquoted outer, 1 backslash) ---

#[test]
fn nested_unquoted_one_backslash_preserved() {
    expect(
        r#"echo [${x:-${y:-'a\$b'}$(printf '')}]"#,
        b"[a\\$b]\n",
        "nested :- (REPORT form) must keep \\$ through both levels",
    );
}

#[test]
fn nested_unquoted_two_backslashes_preserved() {
    expect(
        r#"echo [${x:-${y:-'a\\$b'}$(printf '')}]"#,
        b"[a\\\\$b]\n",
        "nested :- (REPORT form, doubled) must keep both backslashes",
    );
}

#[test]
fn nested_dq_issue_literal_control() {
    expect(
        r#"echo "[${x:-${y:-'a\\$b'}$(printf '')}]""#,
        b"['a\\']\n",
        "issue's literal nested text (double-quoted): GNU gives ['a\\'], not [a\\\\$b]",
    );
}

#[test]
fn nested_dq_one_backslash_control() {
    expect(
        r#"echo "[${x:-${y:-'a\$b'}$(printf '')}]""#,
        b"['a$b']\n",
        "double-quoted nested 1-backslash form: GNU gives ['a$b']",
    );
}

// --- Assignment RHS ---

#[test]
fn assign_comsub_one_backslash_preserved() {
    expect(
        r#"y='a\$b'; x=; v=${x:-$y$(printf '')}; echo "[$v]""#,
        b"[a\\$b]\n",
        "assignment RHS :- with comsub suffix must keep \\$",
    );
}

#[test]
fn assign_comsub_two_backslashes_preserved() {
    expect(
        r#"y='a\\$b'; x=; v=${x:-$y$(printf '')}; echo "[$v]""#,
        b"[a\\\\$b]\n",
        "assignment RHS :- with comsub suffix must keep both backslashes",
    );
}

#[test]
fn assign_nested_one_backslash_preserved() {
    expect(
        r#"v=${x:-${y:-'a\$b'}$(printf '')}; echo "[$v]""#,
        b"[a\\$b]\n",
        "assignment RHS nested :- must keep \\$",
    );
}

#[test]
fn assign_nested_two_backslashes_preserved() {
    expect(
        r#"v=${x:-${y:-'a\\$b'}$(printf '')}; echo "[$v]""#,
        b"[a\\\\$b]\n",
        "assignment RHS nested :- must keep both backslashes",
    );
}

#[test]
fn assign_no_comsub_control() {
    expect(
        r#"y='a\\$b'; x=; v=${x:-$y}; echo "[$v]""#,
        b"[a\\\\$b]\n",
        "assignment RHS :- without comsub was already correct",
    );
}

// --- Other operators ---

#[test]
fn plus_with_comsub_preserved() {
    expect(
        r#"y='a\\$b'; x=; echo "[${x+$y$(printf '')}]""#,
        b"[a\\\\$b]\n",
        ":+ / + alternate must keep both backslashes",
    );
}

#[test]
fn minus_unset_with_comsub_preserved() {
    expect(
        r#"y='a\\$b'; echo "[${x-$y$(printf '')}]""#,
        b"[a\\\\$b]\n",
        "unset - alternate must keep both backslashes",
    );
}

#[test]
fn minus_set_empty_control() {
    expect(
        r#"y='a\\$b'; x=; echo "[${x-$y$(printf '')}]""#,
        b"[]\n",
        "set-empty - uses the (empty) parameter, not the alternate",
    );
}

#[test]
fn plus_colon_empty_control() {
    expect(
        r#"y='a\\$b'; x=; echo "[${x:+$y$(printf '')}]""#,
        b"[]\n",
        "set-empty :+ yields nothing (alternate not used)",
    );
}

// --- Comsub sitting outside the ${} ---

#[test]
fn comsub_after_fragment_preserved() {
    expect(
        r#"y='a\\$b'; x=; echo "[${x:-$y}$(printf '')]""#,
        b"[a\\\\$b]\n",
        "comsub after the fragment must not fold the alternate result",
    );
}

#[test]
fn comsub_after_fragment_space_preserved() {
    expect(
        r#"y='a\\$b'; x=; echo "[${x:-$y} $(printf '')]""#,
        b"[a\\\\$b ]\n",
        "comsub after the fragment (with space) must not fold the result",
    );
}

#[test]
fn comsub_output_after_fragment_preserved() {
    expect(
        r#"y='a\\$b'; x=; echo "[${x:-$y$(printf Z)}]""#,
        b"[a\\\\$bZ]\n",
        "non-empty comsub output after the value must not fold it",
    );
}

// --- Plain-parameter controls ---

#[test]
fn plain_param_dq_control() {
    expect(
        r#"y='a\\$b'; echo "[$y]""#,
        b"[a\\\\$b]\n",
        "plain $y in double quotes was already correct",
    );
}

#[test]
fn plain_param_unquoted_control() {
    expect(
        r#"y='a\$b'; echo [$y]"#,
        b"[a\\$b]\n",
        "plain $y unquoted was already correct",
    );
}

// --- Literal-alternate controls (source escapes, not expansion data) ---

#[test]
fn literal_alternate_dq_control() {
    expect(
        r#"echo "[${x:-'a\\$b'}]""#,
        b"['a\\']\n",
        "literal alternate in double quotes: \\\\ collapses (live escape), $b expands empty",
    );
}

#[test]
fn literal_alternate_backslash_b_control() {
    expect(
        r#"echo "[${x:-'a\\b'}]""#,
        b"['a\\b']\n",
        "literal alternate in double quotes: \\\\ before b collapses to one \\",
    );
}

// --- Nested alternates without a comsub suffix (site C, non-mut path) ---

#[test]
fn nested_unquoted_no_comsub_one_backslash_preserved() {
    expect(
        r#"echo [${x:-${y:-'a\$b'}}]"#,
        b"[a\\$b]\n",
        "nested :- without comsub must keep \\$ through both levels",
    );
}

#[test]
fn nested_unquoted_no_comsub_two_backslashes_preserved() {
    expect(
        r#"echo [${x:-${y:-'a\\$b'}}]"#,
        b"[a\\\\$b]\n",
        "nested :- without comsub must keep both backslashes through both levels",
    );
}

// --- Literal-span backslashes are data, not source escapes (site C) ---

#[test]
fn literal_span_two_backslashes_preserved() {
    expect(
        r#"echo [${x:-'a\\b'}]"#,
        b"[a\\\\b]\n",
        "single-quoted span content is literal data: \\\\ stays doubled",
    );
}

// --- Source escapes must still resolve (regression guards) ---

#[test]
fn source_escape_collapse_control() {
    expect(
        r#"echo [${x:-a\\b}]"#,
        b"[a\\b]\n",
        "source \\\\ in an unquoted alternate still collapses to one \\",
    );
}

#[test]
fn source_escape_q_control() {
    expect(
        r#"echo [${x:-a\qb}]"#,
        b"[aqb]\n",
        "source \\q in an unquoted alternate still drops its backslash",
    );
}

#[test]
fn source_escape_comsub_control() {
    expect(
        r#"echo [${x:-a\qb$(printf '')}]"#,
        b"[aqb]\n",
        "source \\q in an alternate with a comsub suffix still drops its backslash",
    );
}

// --- Same mechanism through other word positions ---

#[test]
fn for_list_dq_preserved() {
    expect(
        r#"y='a\$b'; x=; for i in "${x:-$y}"; do echo "[$i]"; done"#,
        b"[a\\$b]\n",
        "for-list double-quoted alternate must keep \\$",
    );
}

#[test]
fn case_word_dq_preserved() {
    expect(
        r#"y='a\$b'; x=; case "${x:-$y}" in 'a\$b') echo MATCH;; *) echo NOMATCH;; esac"#,
        b"MATCH\n",
        "case word double-quoted alternate must keep \\$ so the pattern matches",
    );
}

// --- Source backslash meeting a $-starting value (site A, DATA_BACKSLASH) ---

#[test]
fn source_double_backslash_dollar_value_preserved() {
    expect(
        r#"y='$b'; x=; echo "[${x:-\\$y}]""#,
        b"[\\$b]\n",
        "source \\\\ plus a value starting with $ must keep the backslash",
    );
}

// --- Heredoc context (same site-A branch) ---

#[test]
fn heredoc_alternate_preserved() {
    expect(
        r#"y='a\$b'; x=; cat <<EOF
[${x:-$y}]
EOF"#,
        b"[a\\$b]\n",
        "heredoc alternate must keep \\$",
    );
}
