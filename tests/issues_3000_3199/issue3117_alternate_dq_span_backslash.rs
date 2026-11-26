//! rubash#3117 — a backslash before a non-escapable character inside a
//! double-quoted span of a `${var:-word}` alternate is data in GNU, but
//! rubash drops it: `${x:-"a\qb"}` yields `[aqb]` instead of `[a\qb]`.
//!
//! GNU: inside double quotes the backslash keeps its escape meaning only
//! before the CBSDQUOTE set — `$`, `` ` ``, `"`, `\`, newline
//! (syntax.h:58). `expand_word_internal`'s backslash branch
//! (subst.c:11614-11678) copies the backslash as literal data when the
//! context is double-quoted and the next character is outside that set
//! (subst.c:11661-11665), and the decision happens in the same single
//! pass that tracks the quote state (subst.c:7966
//! `parameter_brace_expand_rhs` → subst.c:4462 `expand_string_for_rhs` →
//! subst.c:11229 `expand_word_internal`).
//!
//! Rubash: the non-mut alternate path (`expand_parameter_word`,
//! src/executor/parameter_words.rs:291) walks the word with quote state,
//! but the span's `"` characters only toggle walker state — the assembled
//! text loses the span boundary — and the trailing
//! `unescape_unquoted_rhs_escapes` (src/executor/parameter_ops.rs:42)
//! then applies UNQUOTED rules to the whole string, eating the backslash
//! before `q`. Every non-CBSDQUOTE shape in a span is hit the same way.
//!
//! The control cell `dq_span_inside_outer_double_quotes_control` pins a
//! different face on purpose: with the expansion itself inside an outer
//! double-quoted word, GNU gives `[aqb]` — the fix must NOT turn that
//! shape into `[a\qb]`.
//!
//! Expectations are GNU bash 5.3.20 byte output, measured with the probe
//! scripts in `~/workspace/fix-R3116/probes/` (probe3117.sh, probe2.sh,
//! probe3-newline.sh). Base fail-first results are recorded per cell in
//! the lane REPORT (`~/workspace/fix-R3116/REPORT.md` §4).
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

// --- The defect family: non-CBSDQUOTE backslashes inside the span are data ---

#[test]
fn dq_span_q_preserved() {
    expect(
        r#"echo [${x:-"a\qb"}]"#,
        b"[a\\qb]\n",
        "\\q inside a double-quoted alternate span is data: backslash stays",
    );
}

#[test]
fn dq_span_n_preserved() {
    expect(
        r#"echo [${x:-"a\nb"}]"#,
        b"[a\\nb]\n",
        "\\n (letter n) inside the span is data: backslash stays",
    );
}

#[test]
fn dq_span_space_preserved() {
    expect(
        r#"echo [${x:-"a\ b"}]"#,
        b"[a\\ b]\n",
        "backslash-space inside the span is data: backslash stays",
    );
}

#[test]
fn dq_span_squote_preserved() {
    expect(
        r#"echo [${x:-"a\'b"}]"#,
        b"[a\\'b]\n",
        "backslash-squote inside the span is data: backslash stays",
    );
}

#[test]
fn dq_span_star_preserved() {
    expect(
        r#"echo [${x:-"a\*b"}]"#,
        b"[a\\*b]\n",
        "backslash-star inside the span is data: backslash stays",
    );
}

#[test]
fn dq_span_rbrace_preserved() {
    expect(
        r#"echo [${x:-"a\}b"}]"#,
        b"[a\\}b]\n",
        "backslash-rbrace inside the span is data (not in the dq escape set)",
    );
}

#[test]
fn dq_span_two_nonspecial_preserved() {
    expect(
        r#"echo [${x:-"a\q\w"}]"#,
        b"[a\\q\\w]\n",
        "two non-escapable backslashes in one span both stay",
    );
}

#[test]
fn dq_span_q_then_live_expansion() {
    expect(
        r#"b=ZZ; echo [${x:-"a\q$b"}]"#,
        b"[a\\qZZ]\n",
        "\\q stays data while a following real $b still expands",
    );
}

#[test]
fn dq_span_nested_alternate() {
    expect(
        r#"echo [${x:-${y:-"a\qb"}}]"#,
        b"[a\\qb]\n",
        "nested alternate's dq-span keeps its data backslash at both levels",
    );
}

#[test]
fn dq_span_mixed_with_unquoted_text() {
    expect(
        r#"echo [${x:-p"a\qb"s}]"#,
        b"[pa\\qbs]\n",
        "span in the middle of an unquoted alternate keeps its data backslash",
    );
}

#[test]
fn dq_span_with_suffix() {
    expect(
        r#"echo [${x:-"a\qb"}c]"#,
        b"[a\\qbc]\n",
        "text after the expansion does not change span semantics",
    );
}

#[test]
fn dq_span_minus_no_colon() {
    expect(
        r#"echo [${x-"a\qb"}]"#,
        b"[a\\qb]\n",
        "unset - alternate span has the same semantics as :-",
    );
}

#[test]
fn dq_span_plus_colon() {
    expect(
        r#"v=1; echo [${v:+"a\qb"}]"#,
        b"[a\\qb]\n",
        "set :+ alternate span has the same semantics as :-",
    );
}

#[test]
fn dq_span_assign_chain() {
    expect(
        r#"v=${x:-"a\qb"}; echo "[$v]"; echo [${u:-$v}]"#,
        b"[a\\qb]\n[a\\qb]\n",
        "chain: assignment RHS alternate stores a\\qb (intermediate assert), \
         and a later alternate reading $v keeps the data backslash",
    );
}

// --- CBSDQUOTE shapes: the backslash IS an escape inside the span ---

#[test]
fn dq_span_dollar_escape() {
    expect(
        r#"echo [${x:-"a\$b"}]"#,
        b"[a$b]\n",
        "\\$ inside the span is an escape: literal $, no expansion",
    );
}

#[test]
fn dq_span_dollar_escape_beats_set_variable() {
    expect(
        r#"b=ZZ; echo [${x:-"a\$b"}]"#,
        b"[a$b]\n",
        "span \\$ escape wins even when the escaped name is set",
    );
}

#[test]
fn dq_span_dollar_escape_at_span_end() {
    expect(
        r#"echo [${x:-"a\$"}]"#,
        b"[a$]\n",
        "\\$ at the end of the span is a literal $",
    );
}

#[test]
fn dq_span_escape_then_expansion() {
    expect(
        r#"b=ZZ; echo [${x:-"a\$b$b"}]"#,
        b"[a$bZZ]\n",
        "first $ is escaped-literal, the second $b still expands",
    );
}

#[test]
fn dq_span_backslash_backslash() {
    expect(
        r#"echo [${x:-"a\\b"}]"#,
        b"[a\\b]\n",
        "\\\\ inside the span collapses to one literal backslash",
    );
}

#[test]
fn dq_span_dquote_escape() {
    expect(
        r#"echo [${x:-"a\"b"}]"#,
        b"[a\"b]\n",
        "\\\" inside the span is a literal double quote, not the span end",
    );
}

#[test]
fn dq_span_backtick_escape() {
    expect(
        r#"echo [${x:-"a\`b"}]"#,
        b"[a`b]\n",
        "\\` inside the span is a literal backtick, not a comsub trigger",
    );
}

#[test]
fn dq_span_backslash_newline() {
    expect(
        "echo [${x:-\"a\\\nb\"}]",
        b"[ab]\n",
        "backslash-newline inside the span is a line join (both bytes drop)",
    );
}

// --- Controls on other faces ---

#[test]
fn plain_dq_word_control() {
    expect(
        r#"echo "a\qb""#,
        b"a\\qb\n",
        "plain double-quoted word \\q was already correct (not the alternate face)",
    );
}

#[test]
fn dq_span_inside_outer_double_quotes_control() {
    expect(
        r#"echo "[${x:-"a\qb"}]""#,
        b"[aqb]\n",
        "expansion inside an OUTER double-quoted word: GNU itself gives [aqb]; \
         the fix must not turn this shape into [a\\qb]",
    );
}
