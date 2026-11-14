//! rubash#3116 — source `\$` (and the other live-trigger escapes) inside an
//! unquoted `${var:-word}` alternate are not recognised as escapes: the
//! backslash is emitted literally and the escaped character is re-scanned
//! as live syntax, so `\$` expands the following name away.
//!
//! GNU: the alternate word goes through the ordinary single-pass word
//! expansion (subst.c:7966 `parameter_brace_expand_rhs` → subst.c:4462
//! `expand_string_for_rhs` → subst.c:11229 `expand_word_internal`), whose
//! backslash branch (subst.c:11614-11678) consumes `\` before ANY
//! character in an unquoted context and emits the character CTLESC-quoted
//! (subst.c:11671-11674): `\$` is a literal `$` that can never trigger an
//! expansion, `` \` `` a literal backtick, `\$(` literal text.
//!
//! Rubash: `${...}` bodies survive the lexer verbatim, so the word half
//! reaches the non-mut walker raw. That walker's backslash arm
//! (`expand_embedded_parameters_inner`, src/executor/embedded_parameters.rs)
//! only consumes `\'` and `\"` pairs in the unquoted branch; `\$` falls
//! through — the `\` is pushed as text and the `$` is re-scanned as a live
//! expansion trigger. The trailing context-free unescape in
//! `expand_parameter_word` (src/executor/parameter_words.rs:291 →
//! src/executor/parameter_ops.rs:42 `unescape_unquoted_rhs_escapes`) can
//! only fix pairs that survive intact, so `\q` / `\}` / `\\` shapes are
//! correct by accident (regression locks below), while every escaped
//! live trigger is wrong.
//!
//! Expectations are GNU bash 5.3.20 byte output, measured with the probe
//! scripts in `~/workspace/fix-R3116/probes/` (probe3116.sh, probe2.sh,
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

// --- The defect family: escaped live triggers ---

#[test]
fn source_dollar_escape_flat() {
    expect(
        r#"echo [${x:-a\$b}]"#,
        b"[a$b]\n",
        "unquoted alternate \\$ must be a literal $, not an expansion trigger",
    );
}

#[test]
fn source_dollar_escape_beats_set_variable() {
    expect(
        r#"b=ZZ; echo [${x:-a\$b}]"#,
        b"[a$b]\n",
        "escape wins even when the escaped name is set",
    );
}

#[test]
fn source_dollar_escape_with_suffix() {
    expect(
        r#"echo [${x:-a\$b}c]"#,
        b"[a$bc]\n",
        "text after the expansion keeps the literal $ from the escape",
    );
}

#[test]
fn source_dollar_escape_twice() {
    expect(
        r#"echo [${x:-a\$b\$c}]"#,
        b"[a$b$c]\n",
        "two \\$ escapes in one alternate are both literal",
    );
}

#[test]
fn source_backtick_escape() {
    expect(
        r#"echo [${x:-a\`b}]"#,
        b"[a`b]\n",
        "unquoted alternate \\` must be a literal backtick",
    );
}

#[test]
fn source_escaped_comsub_is_literal() {
    expect(
        r#"echo [${x:-\$(echo hi)}]"#,
        b"[$(echo hi)]\n",
        "\\$( must not execute: the whole $(...) stays literal text",
    );
}

#[test]
fn source_escaped_backquote_is_literal() {
    expect(
        r#"echo [${x:-a\`echo hi\`b}]"#,
        b"[a`echo hi`b]\n",
        "escaped backquotes must not execute: literal `echo hi` text",
    );
}

#[test]
fn source_dollar_escape_nested_alternate() {
    expect(
        r#"echo [${x:-${y:-a\$b}}]"#,
        b"[a$b]\n",
        "nested alternate's \\$ is literal at both levels",
    );
}

#[test]
fn source_dollar_escape_minus_no_colon() {
    expect(
        r#"echo [${x-a\$b}]"#,
        b"[a$b]\n",
        "unset - alternate has the same escape semantics as :-",
    );
}

#[test]
fn source_dollar_escape_plus_colon() {
    expect(
        r#"v=1; echo [${v:+a\$b}]"#,
        b"[a$b]\n",
        "set :+ alternate has the same escape semantics as :-",
    );
}

#[test]
fn source_dollar_escape_assign_chain() {
    expect(
        r#"v=${x:-a\$b}; echo "[$v]"; echo [${u:-$v}]"#,
        b"[a$b]\n[a$b]\n",
        "chain: assignment RHS alternate stores a$b (intermediate assert), \
         and a later alternate reading $v keeps it",
    );
}

#[test]
fn source_dollar_escape_assign_rhs() {
    expect(
        r#"w=${x:-p\$q}; echo "[$w]""#,
        b"[p$q]\n",
        "assignment RHS position: \\$ in the alternate is literal",
    );
}

#[test]
fn source_backslash_newline_alternate() {
    expect(
        "echo [${x:-a\\\nb}]",
        b"[ab]\n",
        "backslash-newline inside an unquoted alternate is a line join (both bytes drop)",
    );
}

// --- Regression locks: correct on base only because the escaped char is
// --- not a live walker trigger; the trailing text unescape saves them.

#[test]
fn source_dquote_escape_lock() {
    expect(
        r#"echo [${x:-a\"b}]"#,
        b"[a\"b]\n",
        "\\\" in an unquoted alternate is a literal double quote",
    );
}

#[test]
fn source_backslash_backslash_lock() {
    expect(
        r#"echo [${x:-a\\b}]"#,
        b"[a\\b]\n",
        "\\\\ in an unquoted alternate collapses to one literal backslash",
    );
}

#[test]
fn source_q_escape_lock() {
    expect(
        r#"echo [${x:-a\qb}]"#,
        b"[aqb]\n",
        "source \\q in an unquoted alternate still drops its backslash",
    );
}

#[test]
fn source_rbrace_escape_lock() {
    expect(
        r#"echo [${x:-a\}b}]"#,
        b"[a}b]\n",
        "\\} inside ${} is removed (POSIX issue 221, subst.c:11636-11640)",
    );
}

#[test]
fn source_squote_escape_lock() {
    expect(
        r#"echo [${x:-a\'b}]"#,
        b"[a'b]\n",
        "\\' in an unquoted alternate is a literal single quote",
    );
}

#[test]
fn source_star_escape_lock() {
    expect(
        r#"echo [${x:-a\*b}]"#,
        b"[a*b]\n",
        "\\* in an unquoted alternate is a literal star (no glob)",
    );
}

#[test]
fn source_space_escape_bytes_lock() {
    expect(
        r#"echo [${x:-a\ b}]"#,
        b"[a b]\n",
        "escaped space bytes survive in the alternate result",
    );
}

#[test]
fn source_space_escape_single_field_lock() {
    expect(
        r#"set -- ${x:-a\ b}; echo count=$# first=[$1]"#,
        b"count=1 first=[a b]\n",
        "escaped space keeps the alternate to ONE field (posixexp2 37)",
    );
}

#[test]
fn source_tab_escape_lock() {
    expect(
        "echo [${x:-a\\\tb}]",
        b"[a\tb]\n",
        "backslash-tab in an unquoted alternate is a literal tab, one field",
    );
}

// --- Control: ordinary word position never had the defect (lexer
// --- carrier encodes \$ before the walk).

#[test]
fn plain_word_dollar_escape_control() {
    expect(
        r#"echo a\$b"#,
        b"a$b\n",
        "plain unquoted word \\$ was already correct",
    );
}
