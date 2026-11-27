//! rubash#1870: the printf %b escape path was suspected of lacking
//! the locale gate the #1837 mechanism requires — in a single-byte
//! locale, GNU re-emits `\uNNNN`/`\UNNNNNNNN` escapes literally
//! (canonical form) instead of UTF-8-encoding them
//! (builtins/printf.def -> lib/sh/unicode.c:239 u32cconv under the
//! active locale; locale.c set_default_locale falls back to "C" when
//! no locale variables are set).
//!
//! Triage on master: the LIVE %b path (expand_percent_b ->
//! format_unicode_escape -> unicode_escape_text) is gated on
//! locale::is_multi_byte() and byte-matches the oracle in every cell
//! below. The ungated converter the issue pointed at,
//! format_escape_codepoint (a direct u32cconv_utf8_text call with no
//! locale check), had no callers at all — dead code embodying exactly
//! the ungated behavior. It is deleted so the gated path is the only
//! one available to wire up; these pins lock the behavior.
//!
//! Oracle: GNU bash 5.3.20, byte-compared.

#![cfg(unix)]

use std::process::{Command, Stdio};

fn rubash_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_rubash"))
}

/// Run `script` under rubash with a cleared environment plus `envs`,
/// returning raw stdout bytes.
fn run_with_env(script: &str, envs: &[(&str, &str)]) -> Vec<u8> {
    let dir = std::env::temp_dir().join(format!(
        "rubash-scratch-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let script_path = dir.join("case.sh");
    std::fs::write(&script_path, script).expect("write script");
    let mut command = Command::new(rubash_bin());
    command
        .arg(&script_path)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", &dir)
        .current_dir(&dir)
        .stdin(Stdio::null());
    for (key, value) in envs {
        command.env(key, value);
    }
    let output = command.output().expect("spawn rubash");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        output.status.success(),
        "run must succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

const PERCENT_B_E9: &str = "printf '%b' '\\u00e9'\n";

#[test]
fn issue1870_percent_b_no_locale_vars_is_literal() {
    // No locale variables at all: GNU falls back to the C locale.
    assert_eq!(run_with_env(PERCENT_B_E9, &[]), b"\\u00E9".to_vec());
}

#[test]
fn issue1870_percent_b_c_locale_is_literal() {
    assert_eq!(
        run_with_env(PERCENT_B_E9, &[("LC_ALL", "C")]),
        b"\\u00E9".to_vec()
    );
}

#[test]
fn issue1870_percent_b_utf8_locale_encodes() {
    assert_eq!(
        run_with_env(PERCENT_B_E9, &[("LC_ALL", "C.UTF-8")]),
        vec![0xC3, 0xA9]
    );
}

#[test]
fn issue1870_percent_b_live_locale_switch() {
    // The gate reads the CURRENT locale at each printf: switching
    // LC_ALL mid-script flips the very next %b expansion.
    let script = "LC_ALL=C.UTF-8\nprintf '%b' '\\u00e9'\nLC_ALL=C\nprintf '%b' '\\u00e9'\n";
    let mut expected = vec![0xC3, 0xA9];
    expected.extend_from_slice(b"\\u00E9");
    assert_eq!(run_with_env(script, &[]), expected);
}

#[test]
fn issue1870_format_string_escape_same_gate() {
    // Control: the format-string \u path shares the gate — literal
    // under C, encoded under C.UTF-8.
    let script = "printf '\\u00e9'\n";
    assert_eq!(
        run_with_env(script, &[("LC_ALL", "C")]),
        b"\\u00E9".to_vec()
    );
    assert_eq!(
        run_with_env(script, &[("LC_ALL", "C.UTF-8")]),
        vec![0xC3, 0xA9]
    );
}
