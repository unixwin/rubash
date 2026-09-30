//! Issue rubash#353 regressions: `$'...'` backslash-u / backslash-U
//! escapes decode under the locale in effect when GNU's READER would pull
//! the word's command list - an in-script `export LC_ALL=C` on an earlier
//! line therefore applies, while words in the SAME list (same line,
//! enclosing brace group) and function bodies lexed before the export
//! keep the startup locale.
//!
//! GNU owner: lib/sh/strtrans.c:161-181 ansicstr (backslash-u branch) ->
//! lib/sh/unicode.c:239 u32cconv - values <= 0x7f translate directly; the
//! rest encode per the CURRENT locale, whose non-UTF-8 fallback is
//! u32tocesc (unicode.c:141: backslash-u %04X / backslash-U %08X,
//! UPPERCASE hex). GNU decodes at read time (parse.y:5560 ansiexpand
//! inside read_token_word) - the reader pulls one newline-terminated
//! list at a time.
//!
//! rubash lexes whole files upfront, so the executor re-derives
//! u-bearing words under their line's read-time locale (recorded per
//! source line at each reader-level command start; word metadata carries
//! the lex-time locale; only words from the unit's upfront lex are
//! eligible - eval-lexed words already decoded under their own read-time
//! locale). The re-derivation runs the exact lexer dispatch
//! (lexer::word_value_from_raw) with the decoder locale pinned
//! (locale::with_ansi_lex_locale).
//!
//! Expected outputs byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script files 2026-09-28; probes L1-L8/L4b/L4c
//! under tmp/ in the lane worktree). Known residual (documented in the
//! issue): for-loop `in` words and case words keep the startup-locale
//! decode - the line plumbing reaches command words and assignment RHS
//! values, the dominant family.
//!
//! NOTE on startup locale: with no locale variables set, rubash's
//! platform default is UTF-8 (a documented divergence from GNU's C-locale
//! default), which is the state these tests run in.

use std::io::Write;
use std::process::{Command, Stdio};

fn rubash_stdin_bytes(script: &str) -> (Vec<u8>, Vec<u8>, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("LC_ALL")
        .env_remove("LC_CTYPE")
        .env_remove("LANG")
        .spawn()
        .expect("spawn rubash stdin");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(script.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("collect stdin run");
    (output.stdout, output.stderr, output.status.code())
}

/// L1: `export LC_ALL=C` on an EARLIER line - the printf word's list is
/// read after the export executed, so GNU keeps the literal escape text.
/// GNU output bytes: `\u00E9` + newline (uppercase hex, four digits -
/// u32tocesc uses the `\u%04X` form for any value < 0x10000, even for a
/// `\U` input escape).
#[test]
fn export_then_word_keeps_literal_escape() {
    let script = "export LC_ALL=C\nprintf '%s\\n' $'\\u00e9'\nprintf '%s\\n' $'\\U000000e9'\n";
    let (stdout, stderr, code) = rubash_stdin_bytes(script);
    assert_eq!(
        code,
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&stderr)
    );
    assert_eq!(stdout, b"\\u00E9\n\\u00E9\n");
}

/// L5: the assignment RHS gets the same read-time treatment; the stored
/// variable carries the literal text.
#[test]
fn export_then_assignment_keeps_literal_escape() {
    let script = "export LC_ALL=C\nv=$'\\u00e9'\nprintf '%s\\n' \"$v\"\n";
    let (stdout, stderr, code) = rubash_stdin_bytes(script);
    assert_eq!(
        code,
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&stderr)
    );
    assert_eq!(stdout, b"\\u00E9\n");
}

/// L3: `export LC_ALL=C; printf ...` on ONE line - GNU reads the whole
/// newline-terminated list before executing, so the word keeps the
/// startup (UTF-8) decode: bytes C3 A9.
#[test]
fn same_line_export_does_not_apply() {
    let script = "export LC_ALL=C; printf '%s\\n' $'\\u00e9'\n";
    let (stdout, stderr, code) = rubash_stdin_bytes(script);
    assert_eq!(
        code,
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&stderr)
    );
    assert_eq!(stdout, b"\xc3\xa9\n");
}

/// L2: a brace group is one list - the inner export does not affect the
/// inner word's decode (GNU: UTF-8 bytes).
#[test]
fn brace_group_keeps_list_start_locale() {
    let script = "{ export LC_ALL=C; printf '%s\\n' $'\\u00e9'; }\n";
    let (stdout, stderr, code) = rubash_stdin_bytes(script);
    assert_eq!(
        code,
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&stderr)
    );
    assert_eq!(stdout, b"\xc3\xa9\n");
}

/// L4: a function body defined BEFORE the export was read under the
/// startup locale; calling it later keeps the UTF-8 decode (GNU parity).
#[test]
fn function_defined_before_export_keeps_startup_decode() {
    let script = "f() { printf '%s\\n' $'\\u00e9'; }\nexport LC_ALL=C\nf\n";
    let (stdout, stderr, code) = rubash_stdin_bytes(script);
    assert_eq!(
        code,
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&stderr)
    );
    assert_eq!(stdout, b"\xc3\xa9\n");
}

/// L4c: a function body defined AFTER the export was read under the
/// then-current (C) locale; the call re-derives to the literal text.
#[test]
fn function_defined_after_export_gets_c_decode() {
    let script = "export LC_ALL=C\nf() { printf '%s\\n' $'\\u00e9'; }\nf\n";
    let (stdout, stderr, code) = rubash_stdin_bytes(script);
    assert_eq!(
        code,
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&stderr)
    );
    assert_eq!(stdout, b"\\u00E9\n");
}

/// Values <= 0x7f translate directly in every locale (strtrans.c:174).
#[test]
fn ascii_value_translates_under_c_locale() {
    let script = "export LC_ALL=C\nprintf '%s\\n' $'\\u0041'\n";
    let (stdout, stderr, code) = rubash_stdin_bytes(script);
    assert_eq!(
        code,
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&stderr)
    );
    assert_eq!(stdout, b"A\n");
}

/// With no locale variables the platform default (UTF-8) decodes the
/// escape; the u-escape machinery stays locale-gated, not unconditional.
#[test]
fn no_locale_vars_decode_utf8() {
    let script = "printf '%s\\n' $'\\u00e9'\n";
    let (stdout, stderr, code) = rubash_stdin_bytes(script);
    assert_eq!(
        code,
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&stderr)
    );
    assert_eq!(stdout, b"\xc3\xa9\n");
}

/// An escaped backslash (backslash-backslash before the u) makes the
/// following text literal - it is never a locale-gated escape, in either
/// locale.
#[test]
fn escaped_backslash_u_stays_literal_text() {
    let script = "export LC_ALL=C\nprintf '%s\\n' $'a\\\\u00e9'\n";
    let (stdout, stderr, code) = rubash_stdin_bytes(script);
    assert_eq!(
        code,
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&stderr)
    );
    assert_eq!(stdout, b"a\\u00e9\n");
}

/// A value >= 0x10000 uses the `\U%08X` fallback form under a C locale.
#[test]
fn high_codepoint_uses_capital_u_form() {
    let script = "export LC_ALL=C\nprintf '%s\\n' $'\\U0001f600'\n";
    let (stdout, stderr, code) = rubash_stdin_bytes(script);
    assert_eq!(
        code,
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&stderr)
    );
    assert_eq!(stdout, b"\\U0001F600\n");
}
