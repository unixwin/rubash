//! rubash#3813 — modernish builtin.t 007 解析死亡：别名 `{ f() {` +
//! `$(…)` 内奇数撇号注释致 rc=2。
//!
//! Root cause: the alias pre-pass's `$(...)` extent scanners
//! (`skip_dollar_paren` / `skip_dollar_brace` / `skip_funsub_body` in
//! src/lexer/alias_stream.rs) treated quote characters inside `#`
//! comments and heredoc bodies as real quotes, so an apostrophe in a
//! comment (`# don't`) opened a PHANTOM single-quote that swallowed the
//! rest of the input — and every alias after the substitution with it.
//! The closing alias `E='}; }'` never expanded, leaving the
//! alias-introduced `{` unclosed: `syntax error: unexpected end of file
//! from `{' command`. Even numbers of apostrophes closed the phantom
//! quote again, which is why the parity table flips on odd/even.
//!
//! GNU anchor: parse.y:3631-3645 `shell_getc` — `#` at a shell_getc
//! position ALWAYS comments ("Discard until EOL or EOF"); parse.y:4451
//! `parse_comsub` parses the body with the real parser, so comments
//! and heredocs inside `$(...)` / `${ ... }` are opaque spans.
//! Probed on GNU bash 5.3.20 (~/workspace/rubash-audit/gnu-bash-5.3.20):
//! every case below is rc=0 there except the escaped-`#` control,
//! which is rc=2 on GNU too (the `\#` quotes the `#`, so the apostrophe
//! opens a REAL unclosed quote).
//!
//! The fix gives the three extent scanners the same opaque-span
//! discipline the corrected paren scanner
//! (`skip_parenthesized_unit_corrected`, skip.rs) already carries:
//! `skip_dollar_paren` delegates to it, `skip_dollar_brace` delegates
//! nested `$(...)` to it, and `skip_funsub_body` gains a `#`-comment
//! arm (token-boundary gated) plus a `<<` heredoc arm. Same defect
//! class as #2033's pre-pass walkers, which already carry the comment
//! arm — these three helpers were the residual.
//!
//! NOTE (pin split, 2026-11-17): four pins that need the REAL lexer
//! (`${ ... }` funsub bodies with `#` comments/heredocs, and
//! `${x:-$(...)}` whose closing `}` leaked into the output) were moved
//! out of this file to rubash #3823 — the pre-scan fix here cannot turn
//! them green (no-alias probes: base and fixed binaries behave
//! identically). This file now holds the 11 pre-scan-layer pins only.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("rubash-i3813-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    fn run(&self, script: &str) -> (String, String, i32) {
        let case = self.dir.join("case.sh");
        fs::write(&case, script).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&case)
            .env("HOME", &self.dir)
            .current_dir(&self.dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn alias_frame(body: &str) -> String {
    // `f` is invoked after the group closes: the parse verdict (rc) is
    // the defect signal, but the body must actually RUN for the
    // `v=[...]` stdout pins to mean anything.
    format!(
        "shopt -s expand_aliases\nalias T='{{ f() {{'\nalias E='}}; }}'\nT\n{body}\nE\nf\necho done\n"
    )
}

fn assert_parses(scratch: &Scratch, script: &str, want_stdout: &str) {
    let (out, err, rc) = scratch.run(script);
    assert_eq!(rc, 0, "rc mismatch; stderr:\n{err}");
    assert!(
        !err.contains("syntax error"),
        "unexpected syntax error:\n{err}"
    );
    assert!(out.contains(want_stdout), "stdout mismatch:\n{out}");
    assert!(out.contains("done"), "trailing marker missing:\n{out}");
}

/// Issue's parity table: 0–4 apostrophes in the comsub comment — GNU
/// is rc=0 for all of them; the broken pre-pass failed exactly the odd
/// counts (p1, p3).
#[test]
fn issue3813_parity_apostrophe_count_in_comsub_comment() {
    let scratch = Scratch::new("parity");
    let comments = [
        "",
        "don't",
        "don't can't",
        "don't can't won't",
        "don't can't won't shan't",
    ];
    for (n, comment) in comments.iter().enumerate() {
        let script = alias_frame(&format!("  v=$(echo hi # {comment}\n)\n  echo \"v=[$v]\""));
        let (out, err, rc) = scratch.run(&script);
        assert_eq!(rc, 0, "p{n}: rc mismatch; stderr:\n{err}");
        assert!(out.contains("v=[hi]"), "p{n}: stdout mismatch:\n{out}");
    }
}

/// `#` immediately after `$(` still comments (GNU rc=0).
#[test]
fn issue3813_comment_right_after_comsub_open() {
    let scratch = Scratch::new("open");
    let script = alias_frame("  v=$(# don't\necho hi)\n  echo \"v=[$v]\"");
    assert_parses(&scratch, &script, "v=[hi]");
}

/// Nested `$(...)` with the comment quote in the inner body.
#[test]
fn issue3813_nested_comsub_comment_quote() {
    let scratch = Scratch::new("nested");
    let script = alias_frame("  v=$(echo $(echo hi # don't\n))\n  echo \"v=[$v]\"");
    assert_parses(&scratch, &script, "v=[hi]");
}

/// Comsub inside double quotes: the body is still a command list, so
/// its `#` comments (GNU rc=0).
#[test]
fn issue3813_dquoted_comsub_comment_quote() {
    let scratch = Scratch::new("dquote");
    let script = alias_frame("  v=\"$(echo hi # don't\n)\"\n  echo \"v=[$v]\"");
    assert_parses(&scratch, &script, "v=[hi]");
}

/// Commands after the comment line inside the same `$(...)` still run.
#[test]
fn issue3813_comment_then_command_in_comsub() {
    let scratch = Scratch::new("then");
    let script = alias_frame("  v=$(echo hi # don't\necho yo)\n  echo \"v=[$v]\"");
    assert_parses(&scratch, &script, "v=[hi\nyo]");
}

/// An alias AFTER the comment line inside the comsub body still expands
/// (the #2033 shape, inside the alias-introduced group).
#[test]
fn issue3813_alias_expands_after_comment_in_comsub() {
    let scratch = Scratch::new("after");
    let script = "shopt -s expand_aliases\nalias T='{ f() {'\nalias E='}; }'\nalias IN=echo\nT\n  v=$(echo hi # don't\nIN inner)\n  echo \"v=[$v]\"\nE\nf\necho done\n";
    let (out, err, rc) = scratch.run(script);
    assert_eq!(rc, 0, "rc mismatch; stderr:\n{err}");
    assert!(out.contains("v=[hi\ninner]"), "stdout mismatch:\n{out}");
}

/// Heredoc inside `$(...)` whose body carries an apostrophe: the body is
/// raw text, so the quote must not leak into the extent scan.
#[test]
fn issue3813_heredoc_in_comsub_with_quote() {
    let scratch = Scratch::new("heredoc");
    let script = alias_frame("  v=$(cat <<EOF\nit's here\nEOF\n)\n  echo \"v=[$v]\"");
    assert_parses(&scratch, &script, "v=[it's here]");
}

/// `case` patterns with `)` plus the comment quote inside the comsub.
#[test]
fn issue3813_case_pattern_comsub_comment_quote() {
    let scratch = Scratch::new("case");
    let script =
        alias_frame("  w=a\n  v=$(case $w in (a) echo hi # don't\n;; esac)\n  echo \"v=[$v]\"");
    assert_parses(&scratch, &script, "v=[hi]");
}

/// Controls — these passed before the fix and must keep passing:
/// `$#` is not a comment, `'a'#b` is mid-word, `a#b` is mid-word, and
/// `#` inside `${...}` stays an operator.
#[test]
fn issue3813_hash_controls_stay_literal() {
    let scratch = Scratch::new("controls");
    for (tag, body, want) in [
        ("dollar-hash", "  v=$(echo $#)\n  echo \"v=[$v]\"", "v=[0]"),
        (
            "quoted-hash",
            "  v=$(echo 'a'#b)\n  echo \"v=[$v]\"",
            "v=[a#b]",
        ),
        (
            "midword-hash",
            "  v=$(echo a#b)\n  echo \"v=[$v]\"",
            "v=[a#b]",
        ),
        (
            "brace-hash-operator",
            "  v=$(echo \"${x:-a # b}\")\n  echo \"v=[$v]\"",
            "v=[a # b]",
        ),
    ] {
        let script = alias_frame(body);
        let (out, err, rc) = scratch.run(&script);
        assert_eq!(rc, 0, "{tag}: rc mismatch; stderr:\n{err}");
        assert!(out.contains(want), "{tag}: stdout mismatch:\n{out}");
    }
}

/// Escaped `\#` quotes the `#`, so the apostrophe opens a REAL unclosed
/// quote: GNU is rc=2 here too. Only the exit code is pinned (rubash's
/// diagnostic names the `{` first, GNU's the quote — a separate face).
#[test]
fn issue3813_escaped_hash_still_errors() {
    let scratch = Scratch::new("escaped");
    let script = alias_frame("  v=$(echo hi \\# don't)\n  echo \"v=[$v]\"");
    let (_out, err, rc) = scratch.run(&script);
    assert_eq!(rc, 2, "rc mismatch; stderr:\n{err}");
    assert!(
        err.contains("matching"),
        "expected the unclosed-quote diagnostic:\n{err}"
    );
}

/// No-alias control: the literal `{ f() {` group with the same comsub
/// parsed before the fix and must keep parsing.
#[test]
fn issue3813_noalias_literal_group_passes() {
    let scratch = Scratch::new("noalias");
    let script = "{ f() {\n  v=$(echo hi # don't\n)\n  echo \"v=[$v]\"\n}\n}\nf\necho done\n";
    let (out, err, rc) = scratch.run(script);
    assert_eq!(rc, 0, "rc mismatch; stderr:\n{err}");
    assert!(out.contains("v=[hi]"), "stdout mismatch:\n{out}");
}
