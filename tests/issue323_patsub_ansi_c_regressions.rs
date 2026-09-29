//! Issue rubash#323: ANSI-C escapes were not decoded in the replacement
//! operand of `${var//pat/$'\n'}` — `$'\n'`/`$'\t'` (and every C escape
//! besides `\xHH`, `\'`, `\\`) stored literal backslash-letter text, so
//! pbb's "Split a string on a delimiter" (section 004) emitted
//! `apples\noranges\npears` as ONE line.
//!
//! GNU semantics (vendored third_party/bash): the replacement side of a
//! pattern substitution is expanded as its own word — subst.c:4462
//! expand_string_for_rhs (from parameter_brace_patsub) hands it through
//! call_expand_word_internal, whose `$'` handling decodes with ansicstr
//! (lib/sh/strtrans.c:51): `\a \b \e \E \f \n \r \t \v`, octal `\NNN`,
//! `\xHH`, `\uNNNN`/`\UNNNNNNNN`, `\cX`, quote escapes; UNRECOGNIZED
//! escapes keep the backslash (`$'\q'` → `\q`).
//!
//! Root cause: rubash's patsub replacement marking pass
//! (expand_braced_replacement.rs push_ansi_c_replacement_span) carried a
//! hand-rolled decoder that only knew `\xHH`, `\'` and `\\`, dumping every
//! other escape through the "unrecognized" arm.
//!
//! Fix: the span now decodes its body through the SAME canonical decoder
//! as every other `$'...'` context (lexer ansi.rs decode_ansi_c_quoted,
//! the ansicstr port).
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-28).

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

fn assert_clean(stderr: &str) {
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
}

/// The reporter's exact shape: `$'\n'` decodes to a real newline.
#[test]
fn newline_replacement_decodes() {
    let (stdout, stderr, _) =
        rubash("s=\"a,b,c\"\nr=\"${s//,/$'\\n'}\"\nprintf '%s' \"$r\" | od -An -c");
    assert_clean(&stderr);
    assert_eq!(stdout, "   a  \\n   b  \\n   c\n");
}

/// `$'\t'` decodes to a real tab.
#[test]
fn tab_replacement_decodes() {
    let (stdout, stderr, _) =
        rubash("s=\"a,b,c\"\nr=\"${s//,/$'\\t'}\"\nprintf '%s' \"$r\" | od -An -c");
    assert_clean(&stderr);
    assert_eq!(stdout, "   a  \\t   b  \\t   c\n");
}

/// The control-escape family decodes: `\r`, `\a`, `\v`.
#[test]
fn control_escapes_decode() {
    let (stdout, stderr, _) = rubash(
        "s=aXb\nprintf '%s' \"${s//X/$'\\r'}\" | od -An -c | head -1\n\
         printf '%s' \"${s//X/$'\\a'}\" | od -An -c | head -1\n\
         printf '%s' \"${s//X/$'\\v'}\" | od -An -c | head -1",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "   a  \\r   b\n   a  \\a   b\n   a  \\v   b\n");
}

/// Octal `\012` (newline), `\101` (`A`) and control `\cA` forms.
#[test]
fn octal_and_control_forms_decode() {
    let (stdout, stderr, _) = rubash(
        "s=aXb\nprintf '%s' \"${s//X/$'\\101'}\" | od -An -c | head -1\n\
         printf '%s' \"${s//X/$'\\cA'}\" | od -An -c | head -1",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "   a   A   b\n   a 001   b\n");
}

/// `\x2e` still decodes (it worked before; must not regress).
#[test]
fn hex_escape_still_decodes() {
    let (stdout, stderr, _) =
        rubash("s=aXb\nprintf '%s' \"${s//X/$'\\x2e'}\" | od -An -c | head -1");
    assert_clean(&stderr);
    assert_eq!(stdout, "   a   .   b\n");
}

/// GNU passthrough: an UNRECOGNIZED escape keeps the backslash
/// (`$'\q'` → `\q`); escaped quote/backslash decodes; `&` stays data
/// when the patsub_replacement shopt would otherwise expand it.
#[test]
fn unrecognized_escape_keeps_backslash() {
    let (stdout, stderr, _) = rubash("s=aXb\nprintf '%s' \"${s//X/$'\\q'}\"; echo");
    assert_clean(&stderr);
    assert_eq!(stdout, "a\\qb\n");
}

#[test]
fn escaped_quote_and_ampersand_decode() {
    let (stdout, stderr, _) = rubash(
        "s=aXb\nprintf '%s' \"${s//X/$'\\\\&'}\"; echo\nprintf '%s' \"${s//X/$'\\\\\\''}\"; echo",
    );
    assert_clean(&stderr);
    // `$'\\&'` → `\&`; `$'\\\''` → `\'` (backslash, then escaped quote).
    assert_eq!(stdout, "a\\&b\na\\'b\n");
}

/// Unicode `\u00e9` in the replacement decodes.
#[test]
fn unicode_escape_decodes() {
    let (stdout, stderr, _) = rubash("s=aXb\nprintf '%s' \"${s//X/$'\\u00e9'}\"; echo");
    assert_clean(&stderr);
    assert_eq!(stdout, "aéb\n");
}

/// A plain DOUBLE-QUOTED variable replacement containing backslash-t
/// stays literal (no ANSI-C decoding of ordinary parameter values; an
/// unquoted `v=x\ty` would have the backslash escape-removed instead).
#[test]
fn plain_variable_replacement_stays_literal() {
    let (stdout, stderr, _) =
        rubash("s=aXb\nv=\"x\\ty\"\nprintf '%s' \"${s//X/$v}\" | od -An -c | head -1");
    assert_clean(&stderr);
    assert_eq!(stdout, "   a   x   \\   t   y   b\n");
}

/// The pbb split() idiom end-to-end: `${str//$delim/$'\n'}` produces a
/// string with real newlines.
#[test]
fn pbb_split_idiom_newline_replacement() {
    let (stdout, stderr, _) = rubash(
        "s=\"a,b,c\"\nr=\"${s//,/$'\\n'}\"\nprintf '%s\\n' \"${#r}\"\nprintf '%s' \"$r\" | wc -l",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "5\n2\n");
}

/// `${var@Q}` rendering of a decoded replacement round-trips through the
/// $'...' quoting form.
#[test]
fn quoted_expansion_form_round_trips() {
    let (stdout, stderr, _) =
        rubash("s=\"a,b,c\"\nr=\"${s//,/$'\\n'}\"\nprintf '%s\\n' \"${r@Q}\"");
    assert_clean(&stderr);
    assert_eq!(stdout, "$'a\\nb\\nc'\n");
}
