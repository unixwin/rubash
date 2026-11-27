//! Issue rubash#1060: a user-literal U+E000 (PUA plane-1 private use) was
//! stripped on quoting/format paths. Two layers collided:
//!
//! 1. The raw-byte-marker decoders (`shell_text_to_raw_bytes`,
//!    `decode_raw_byte_markers`, `decode_raw_byte_markers_to_byte_chars`,
//!    `decode_to_visible_text`) treated `E000 + <non-payload char>` as a
//!    "stray introducer" and dropped the E000 — but a literal user E000
//!    produces exactly that shape. GNU has no sentinel concept; the char
//!    is data and must pass through.
//! 2. `value.contains(E000)` quote gates (`shell_reusable_quote`,
//!    `quote_declare_value`, `quote_array_element_value`,
//!    `shell_quote_form`, `word_needs_ansic_quote`) routed any value
//!    holding E000 to the byte-octal `$'...'` path even when the E000 is
//!    a printable literal char (GNU strtrans.c ansic_shouldquote:341 and
//!    sh_double_quote print it verbatim inside `"..."`).
//!    The gate is now `has_raw_byte_marker_pair` — E000 followed by a
//!    real payload char (E001..=E100) only.
//!
//! GNU references:
//! - strtrans.c:341-361 ansic_shouldquote (printability test, iswprint
//!   for wide chars — U+E000 is printable under a UTF-8 locale)
//! - strtrans.c:230-308 ansic_quote; shquote.c sh_backslash_quote (%q)
//! - variables.c show_var_attributes -> setattr.def:528-531 declare -p
//!
//! Baselines verified against WSL GNU bash 5.3.20.

#![cfg(windows)]

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn run_script(script: &str) -> (Option<i32>, String, String) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1060-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue1060 scratch dir");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash script");
    let _ = std::fs::remove_dir_all(&dir);
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// printf `\U0000E000` emits the UTF-8 encoding of U+E000 verbatim —
/// neither the char nor the following byte is eaten.
#[test]
fn printf_unicode_escape_emits_literal_e000() {
    // WSL GNU 5.3.20: ` 61 ee 80 80 62`
    let (_, stdout, _) = run_script("printf \"a\\\\U0000E000b\" | od -An -tx1\n");
    assert_eq!(stdout.trim(), "61 ee 80 80 62");
}

/// declare -p renders a value holding literal U+E000 in the double-quote
/// form with the char verbatim (printable PUA, not a marker).
#[test]
fn declare_p_preserves_literal_e000() {
    let e000 = '\u{E000}';
    // WSL GNU 5.3.20: declare -- y="a<E000>b"
    let (_, stdout, _) = run_script("x=$'\\uE000'; y=\"a${x}b\"; declare -p y\n");
    assert_eq!(stdout, format!("declare -- y=\"a{e000}b\"\n"));

    // Lone / trailing / leading positions all keep the char.
    let (_, stdout, _) =
        run_script("x=$'\\uE000'; z=\"${x}\"; w=\"${x}T\"; v=\"T${x}\"; declare -p z w v\n");
    assert_eq!(
        stdout,
        format!("declare -- z=\"{e000}\"\ndeclare -- w=\"{e000}T\"\ndeclare -- v=\"T{e000}\"\n")
    );
}

/// `printf %q` and `${var@Q}` render the printable PUA char, not `$'...'`.
#[test]
fn quote_forms_preserve_literal_e000() {
    let e000 = '\u{E000}';
    // WSL GNU 5.3.20: %q emits the bare char; @Q single-quotes it.
    let (_, stdout, _) =
        run_script("y=\"a${x}b\"; x=$'\\uE000'; printf 'q=[%q]\\n' \"a${x}b\"; echo \"${x@Q}\"\n");
    assert_eq!(stdout, format!("q=[a{e000}b]\n'{e000}'\n"));
}

/// A genuine raw-byte marker pair still takes the octal `$'...'` path —
/// `x=$'\x81'` stores byte 0x81 as E000+E081 and prints `$'\201'`.
#[test]
fn real_raw_byte_pair_still_octal_quotes() {
    // WSL GNU 5.3.20: declare -- x=$'\201'
    let (_, stdout, _) = run_script("x=$'\\x81'; declare -p x\n");
    assert_eq!(stdout, "declare -- x=$'\\201'\n");
}
