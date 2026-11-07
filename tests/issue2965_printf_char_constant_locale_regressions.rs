//! rubash#2965: `printf %d "'é"` in a single-byte locale must yield the
//! FIRST BYTE of the character (195 for U+00E9's 0xC3), not its code
//! point (233). GNU's numeric getters route a leading-quote operand to
//! asciicode() (builtins/printf.def:1522-1547), which converts arg[1]
//! with mbrtowc in the CURRENT locale: in a single-byte locale that is
//! the byte itself; in a multibyte locale the decoded character; on an
//! invalid/incomplete decode (mbrtowc -1/-2, include/shmbutil.h:44
//! MB_INVALIDCH) it falls back to `(unsigned char)arg[1]`.
//!
//! The quote is special only as the operand's FIRST byte
//! (printf.def:1416 getintmax and siblings): `+'a` / `-'a` / ` 'a` go
//! to strtoimax and report `invalid number`. %c/%s/%b have no
//! character-constant form at all: getchr (printf.def:1351-1366)
//! returns arg[0] (a raw byte) and getstr returns the argument
//! verbatim.
//!
//! The locale in effect is the shell's, including an in-script
//! `LC_ALL=...` assignment (variables.c sv_lc_all watcher), not only
//! the process environment (the rubash#1488 guard mechanism).
//!
//! Oracle: GNU bash 5.3.20, byte-compared under LC_ALL=C and
//! LC_ALL=C.UTF-8.

#![cfg(unix)]

use std::process::{Command, Stdio};

fn rubash_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_rubash"))
}

struct Outcome {
    status: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// Run `script` (as a file) under rubash with a cleared environment
/// plus `envs`, returning status/stdout/stderr as raw bytes.
fn run_with_env(script: &str, envs: &[(&str, &str)]) -> Outcome {
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
    Outcome {
        status: output.status.code().unwrap_or(-1),
        stdout: output.stdout,
        stderr: output.stderr,
    }
}

fn stdout_in(script: &str, locale: &str) -> Vec<u8> {
    let outcome = run_with_env(script, &[("LC_ALL", locale)]);
    assert_eq!(
        outcome.status,
        0,
        "run must succeed, stderr: {}",
        String::from_utf8_lossy(&outcome.stderr)
    );
    outcome.stdout
}

const C: &str = "C";
const UTF8: &str = "C.UTF-8";

#[test]
fn issue2965_c_locale_takes_first_byte() {
    assert_eq!(stdout_in("printf '%d\\n' \"'é\"\n", C), b"195\n".to_vec());
    assert_eq!(stdout_in("printf '%d\\n' \"'ü\"\n", C), b"195\n".to_vec());
    assert_eq!(stdout_in("printf '%d\\n' \"'Ā\"\n", C), b"196\n".to_vec());
}

#[test]
fn issue2965_c_locale_first_byte_euro_emoji_two_args() {
    assert_eq!(
        stdout_in("printf '%d %d\\n' \"'é\" \"'€\"\n", C),
        b"195 226\n".to_vec()
    );
    assert_eq!(stdout_in("printf '%d\\n' \"'😀\"\n", C), b"240\n".to_vec());
}

#[test]
fn issue2965_c_locale_double_quote_prefix() {
    assert_eq!(stdout_in("printf '%d\\n' '\"é'\n", C), b"195\n".to_vec());
}

#[test]
fn issue2965_c_locale_all_int_conversions_share_asciicode() {
    assert_eq!(
        stdout_in("printf '%x %o %u %i\\n' \"'é\" \"'é\" \"'é\" \"'é\"\n", C),
        b"c3 303 195 195\n".to_vec()
    );
}

#[test]
fn issue2965_c_locale_float_char_constant() {
    assert_eq!(
        stdout_in("printf '%f\\n' \"'é\"\n", C),
        b"195.000000\n".to_vec()
    );
}

#[test]
fn issue2965_utf8_locale_takes_code_point() {
    assert_eq!(
        stdout_in("printf '%d\\n' \"'é\"\n", UTF8),
        b"233\n".to_vec()
    );
    assert_eq!(
        stdout_in("printf '%d\\n' \"'€\"\n", UTF8),
        b"8364\n".to_vec()
    );
    assert_eq!(
        stdout_in("printf '%d\\n' \"'😀\"\n", UTF8),
        b"128512\n".to_vec()
    );
    assert_eq!(
        stdout_in("printf '%f\\n' \"'é\"\n", UTF8),
        b"233.000000\n".to_vec()
    );
}

#[test]
fn issue2965_utf8_locale_invalid_bytes_fall_back_to_first_byte() {
    // mbrtowc fails (-1 invalid / -2 incomplete) on a lone 0xFF, a lone
    // continuation 0x80 and a truncated 0xC3: asciicode falls back to
    // (unsigned char)arg[1] even in a multibyte locale.
    assert_eq!(
        stdout_in("printf '%d\\n' \"'$(printf '\\xff')\"\n", UTF8),
        b"255\n".to_vec()
    );
    assert_eq!(
        stdout_in("printf '%d\\n' \"'$(printf '\\x80')\"\n", UTF8),
        b"128\n".to_vec()
    );
    assert_eq!(
        stdout_in("printf '%d\\n' \"'$(printf '\\xc3')\"\n", UTF8),
        b"195\n".to_vec()
    );
}

#[test]
fn issue2965_c_locale_high_bytes_are_themselves() {
    assert_eq!(
        stdout_in("printf '%d\\n' \"'$(printf '\\xff')\"\n", C),
        b"255\n".to_vec()
    );
    assert_eq!(
        stdout_in("printf '%d\\n' \"'$(printf '\\x80')\"\n", C),
        b"128\n".to_vec()
    );
}

#[test]
fn issue2965_ascii_controls_agree_in_both_locales() {
    for locale in [C, UTF8] {
        assert_eq!(
            stdout_in("printf '%d\\n' \"'a\"\n", locale),
            b"97\n".to_vec()
        );
        // Only the first character is read; trailing text is ignored.
        assert_eq!(
            stdout_in("printf '%d\\n' \"'abc\"\n", locale),
            b"97\n".to_vec()
        );
        // A lone quote reads the NUL at arg[1]: 0, no diagnostic.
        assert_eq!(stdout_in("printf '%d\\n' \"'\"\n", locale), b"0\n".to_vec());
    }
}

#[test]
fn issue2965_sign_or_space_before_quote_is_invalid_number() {
    for locale in [C, UTF8] {
        for operand in ["+'a", "-'a", " 'a"] {
            let script = format!("printf '%d\\n' \"{operand}\"\n");
            let outcome = run_with_env(&script, &[("LC_ALL", locale)]);
            assert_eq!(outcome.status, 1, "operand {operand}");
            assert_eq!(outcome.stdout, b"0\n".to_vec(), "operand {operand}");
            let stderr = String::from_utf8_lossy(&outcome.stderr);
            assert!(
                stderr.contains(&format!("printf: {operand}: invalid number")),
                "operand {operand}, stderr: {stderr}"
            );
        }
    }
}

#[test]
fn issue2965_percent_c_takes_first_byte_not_char_constant() {
    for locale in [C, UTF8] {
        // No character-constant form for %c: getchr returns arg[0], so
        // the quote itself is printed for a quoted operand...
        assert_eq!(stdout_in("printf '%c' \"'a\"\n", locale), b"'".to_vec());
        // ...and a multibyte argument yields its first raw byte (0xC3),
        // never the whole character, in either locale.
        assert_eq!(stdout_in("printf '%c' 'é'\n", locale), vec![0xC3]);
    }
}

#[test]
fn issue2965_percent_s_and_b_take_the_argument_literally() {
    for locale in [C, UTF8] {
        assert_eq!(
            stdout_in("printf '%s|%b\\n' \"'a\" \"'a\"\n", locale),
            b"'a|'a\n".to_vec()
        );
    }
}

#[test]
fn issue2965_in_script_lc_all_assignment_governs() {
    // variables.c sv_lc_all: an in-script assignment (exported or not)
    // changes the shell's locale semantics at once.
    let to_c = "LC_ALL=C\nprintf '%d\\n' \"'é\"\n";
    assert_eq!(stdout_in(to_c, UTF8), b"195\n".to_vec());
    let to_c_exported = "export LC_ALL=C\nprintf '%d\\n' \"'é\"\n";
    assert_eq!(stdout_in(to_c_exported, UTF8), b"195\n".to_vec());
    let to_utf8 = "LC_ALL=C.UTF-8\nprintf '%d\\n' \"'é\"\n";
    assert_eq!(stdout_in(to_utf8, C), b"233\n".to_vec());
}

#[test]
fn issue2965_pipeline_stage_char_constant() {
    assert_eq!(
        stdout_in("printf '%d\\n' \"'é\" | cat\n", C),
        b"195\n".to_vec()
    );
    assert_eq!(
        stdout_in("printf '%d\\n' \"'é\" | cat\n", UTF8),
        b"233\n".to_vec()
    );
}
