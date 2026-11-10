//! rubash#2772 (deep-audit B4-F02) — `extquote` OFF was ignored inside
//! a double-quoted `${...}` word: the `$'...'` in a default word was
//! ANSI-C decoded at expansion time even when the word was READ after
//! `shopt -u extquote` took effect.
//!
//! GNU anchors (bash 5.3): the verdict is taken at READ time.
//! - parse.y:4053 `parse_matched_pair`: a `$'...'` span inside a
//!   `${ }` body is translated while reading iff `extended_quote` is
//!   set at that moment, OR the word is not double-quoted
//!   ((rflags & P_DQUOTE) == 0), OR the dolbrace state is
//!   DOLBRACE_QUOTE / DOLBRACE_QUOTE2 (parser.h:77-78) — the pattern
//!   positions of `#`/`%`/`^`/`,` (parse.y:4010-4023) and the
//!   pattern/replacement of `/` (parse.y:4016-4017). The default /
//!   alternate / error word positions (DOLBRACE_WORD) get NO such
//!   exemption: inside double quotes with extquote off they stay
//!   literal.
//! - subst.c:1586-1610 is the extraction-time twin gate.
//! - Read time is per READ UNIT: a whole line is parsed before any of
//!   it executes, so `shopt -u extquote; echo "..."` on ONE line still
//!   decodes (the shopt has not run when the line is read), while the
//!   same two commands on separate lines do not.
//!
//! Oracle: GNU bash 5.3.20 probe scripts (2026-11-08), byte-compared
//! (od-verified TAB vs literal backslash-t).
//! Harness: script FILES via CARGO_BIN_EXE_rubash, LC_ALL=C.UTF-8.

use std::process::{Command, Stdio};

struct Outcome {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    code: i32,
}

fn run_file(script: &str) -> Outcome {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "rubash_issue2772_{}_{}_{}.sh",
        std::process::id(),
        script.len(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    std::fs::write(&path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_file(&path);
    Outcome {
        stdout: output.stdout,
        stderr: output.stderr,
        code: output.status.code().unwrap_or(-1),
    }
}

fn expect(script: &str, want_stdout: &[u8], ctx: &str) {
    let out = run_file(script);
    assert_eq!(
        out.stdout,
        want_stdout,
        "{ctx}: stdout; stderr {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.code, 0, "{ctx}: exit code");
}

#[test]
fn off_next_line_dquoted_default_word_stays_literal() {
    expect(
        "shopt -u extquote\necho \"pre ${undef:-$'X\\tY'} post\"\n",
        b"pre $'X\\tY' post\n",
        "extquote off, next line, double-quoted default word",
    );
}

#[test]
fn off_same_line_dquoted_default_word_decodes() {
    expect(
        "shopt -u extquote; echo \"pre ${undef:-$'X\\tY'} post\"\n",
        b"pre X\tY post\n",
        "extquote off on the SAME line: read happened first",
    );
}

#[test]
fn on_next_line_dquoted_default_word_decodes() {
    expect(
        "shopt -s extquote\necho \"pre ${undef:-$'X\\tY'} post\"\n",
        b"pre X\tY post\n",
        "extquote explicitly on (control)",
    );
}

#[test]
fn default_on_dquoted_default_word_decodes() {
    expect(
        "echo \"pre ${undef:-$'X\\tY'} post\"\n",
        b"pre X\tY post\n",
        "extquote default on (control)",
    );
}

#[test]
fn off_next_line_unquoted_default_word_decodes() {
    // Not double-quoted: parse.y:4053 translates regardless of
    // extended_quote ((rflags & P_DQUOTE) == 0).
    expect(
        "shopt -u extquote\necho unquoted: ${undef:-$'X\\tY'}\n",
        b"unquoted: X\tY\n",
        "extquote off, unquoted default word",
    );
}

#[test]
fn off_next_line_dquoted_alternate_word_stays_literal() {
    expect(
        "s=1\nshopt -u extquote\necho \"alt: pre ${s+$'Q\\tR'} post\"\n",
        b"alt: pre $'Q\\tR' post\n",
        "extquote off, double-quoted alternate word",
    );
}

#[test]
fn off_next_line_pattern_position_decodes() {
    // `#` moves the dolbrace state to DOLBRACE_QUOTE (parse.y:4013):
    // single quotes are special there, extquote does not gate it.
    expect(
        "v=abcdef\nshopt -u extquote\necho \"pat: ${v#$'abc'}\"\n",
        b"pat: def\n",
        "extquote off, pattern position",
    );
}

#[test]
fn off_next_line_patsub_replacement_decodes() {
    // `/` moves the dolbrace state to DOLBRACE_QUOTE2 (parse.y:4017).
    expect(
        "v=abcdef\nshopt -u extquote\necho \"rep: ${v/b/$'Z\\tW'}\"\n",
        b"rep: aZ\tWcdef\n",
        "extquote off, patsub replacement position",
    );
}

#[test]
fn off_next_line_heredoc_default_word_stays_literal() {
    expect(
        "shopt -u extquote\ncat <<HD\nheredoc: pre ${undef2:-$'X\\tY'} post\nHD\n",
        b"heredoc: pre $'X\\tY' post\n",
        "extquote off, heredoc default word",
    );
}

#[test]
fn function_body_read_while_on_decodes_when_called_while_off() {
    // The body is one read unit, parsed at definition time with
    // extquote still on; calling it after `shopt -u` must NOT
    // re-judge the $'...' — GNU prints the decoded TAB (probe
    // q2772.sh, 2026-11-08). This cell is what rules out an
    // expansion-time shopt check as the fix.
    expect(
        "f() { echo \"funcbody: pre ${undef:-$'X\\tY'} post\"; }\nshopt -u extquote\nf\n",
        b"funcbody: pre X\tY post\n",
        "extquote verdict is per read unit (definition)",
    );
}

#[test]
fn eval_read_while_off_stays_literal() {
    // eval re-reads its text when it runs: with extquote off at that
    // read, the double-quoted default word stays literal (probe
    // q2772.sh, 2026-11-08).
    expect(
        "shopt -u extquote\neval 'echo \"evalform: pre ${undef:-$'\"'\"'X\\tY'\"'\"'} post\"'\n",
        b"evalform: pre $'X\\tY' post\n",
        "extquote verdict is per read unit (eval)",
    );
}

#[test]
fn off_toggled_back_on_next_line_decodes() {
    expect(
        "shopt -u extquote\nshopt -s extquote\necho \"pre ${undef:-$'X\\tY'} post\"\n",
        b"pre X\tY post\n",
        "off then on again before the read",
    );
}
