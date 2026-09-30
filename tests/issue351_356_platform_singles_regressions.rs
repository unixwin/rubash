//! Issues rubash#351/#353/#356 — heredoc delimiter quote, locale \u gate,
//! nameref error rendering.
//!
//! #351: `cat <<'E` (unterminated quote in the here-document delimiter) was
//! silently accepted — warning + rc=0. GNU read_token_word (parse.y:5419-
//! 5437) reads the delimiter as an ordinary WORD, so the open quote keeps
//! the read going to EOF and fails: `unexpected EOF while looking for
//! matching `'` rc=2. The blanket heredoc exclusion on the driver's
//! unclosed-diagnostics route (`!input.contains("<<")`) hid it; the
//! delimiter shape is now detected and reported before that guard, and the
//! heredoc declaration scanner never declares a delimiter with an open
//! quote (heredoc BODIES stay literal — an unclosed quote there is data).
//!
//! #353: `$'\u00e9'` decoded UTF-8 regardless of locale. GNU strtrans.c
//! :161-181: values <= 0x7f translate directly, everything else goes
//! through u32cconv, whose non-UTF-8-locale fallback is u32tocesc
//! (unicode.c:141: `\u%04X` / `\U%08X` — ISO C99 escape, UPPERCASE hex).
//! The gate closes for an EXPLICIT non-UTF-8 locale; with no locale
//! variables rubash's platform default stays UTF-8 (Windows text layer).
//! In-script `export LC_ALL=C` cannot reach the lexer: the script file is
//! tokenized (and $'...' decoded) before the export line executes — the
//! external-environment form is the testable equivalent.
//!
//! #356: `declare -a a=(zero); declare -n a='(one two three)'` reported
//! the invalid-nameref value through the typed array serialization
//! `("one"  "two" "three")'. GNU declare.def:576 prints the RAW value;
//! the diagnostic now decodes the storage words back to their text.
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

/// #351: unclosed quote in the heredoc delimiter fails at EOF.
#[test]
fn unclosed_quote_in_heredoc_delimiter_is_eof_error() {
    let (stdout, stderr, code) = rubash("cat <<'E\nE");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(2));
    assert!(
        stderr.contains("line 1: unexpected EOF while looking for matching `''"),
        "stderr: {stderr}"
    );
}

/// #351 guard: heredoc bodies are literal — an unclosed quote in the BODY
/// is data and the heredoc completes normally (heredoc.tests regression).
#[test]
fn unclosed_quote_in_heredoc_body_is_data() {
    let (stdout, stderr, code) = rubash("cat <<EOF\necho \"\nEOF\necho done");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "echo \"\ndone\n");
}

/// #351 guard: ordinary (closed) quoted delimiters still suppress body
/// expansion.
#[test]
fn quoted_delimiter_heredoc_still_works() {
    let (stdout, _, code) = rubash("cat <<'X'\n$a\nX");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "$a\n");
}

/// #353: under an explicit C locale the \u escape stays an ISO C99 escape
/// with UPPERCASE hex (external environment — the testable form; the
/// process env is the lexer's only locale view because script files are
/// tokenized before any in-script export runs).
#[test]
fn explicit_c_locale_keeps_u_escape_literal() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("LC_ALL", "C")
        .arg("-c")
        .arg(r"printf '%s\n' $'\u00e9'")
        .output()
        .expect("run rubash");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "\\u00E9\n");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("LC_ALL", "C")
        .arg("-c")
        .arg(r"printf '%s\n' $'\U000000e9'")
        .output()
        .expect("run rubash");
    // u32tocesc formats < 0x10000 as \u regardless of the introducer.
    assert_eq!(String::from_utf8_lossy(&output.stdout), "\\u00E9\n");
}

/// #353 guards: ASCII values decode directly in every locale; UTF-8
/// locales decode normally.
#[test]
fn ascii_and_utf8_locale_decode_normally() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("LC_ALL", "C")
        .arg("-c")
        .arg(r"printf '%s' $'\u0041'")
        .output()
        .expect("run rubash");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "A");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(r"printf '%s' $'\u00e9'")
        .output()
        .expect("run rubash");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "é");
}

/// #356: the invalid-nameref value prints the RAW text, not the array
/// serialization.
#[test]
fn nameref_invalid_value_error_prints_raw_text() {
    let (stdout, stderr, code) =
        rubash("declare -a array=(zero)\ndeclare -n array='(one two three)'");
    assert_eq!(stdout, "");
    assert_ne!(code, Some(0));
    assert!(
        stderr.contains("declare: `(one two three)': invalid variable name for name reference"),
        "stderr: {stderr}"
    );
}

/// #356 guard: the scalar-path rendering (no pre-existing array) is
/// unchanged.
#[test]
fn nameref_invalid_value_scalar_path_unchanged() {
    let (_, stderr, code) =
        rubash("declare array='(one two three)'\ndeclare -n array='(one two three)'");
    assert_ne!(code, Some(0));
    assert!(
        stderr.contains("declare: `(one two three)': invalid variable name for name reference"),
        "stderr: {stderr}"
    );
}
