//! Pin for the #3630 audit finding: GLOBIGNORE matching must honour GNU's
//! `xstrmatch` locale dispatch (smatch.c:585-603). Under a multibyte
//! (UTF-8) locale with multibyte data present, GNU runs
//! `internal_wstrmatch` — `?` and `[...]` consume one CHARACTER — while a
//! single-byte locale stays on `internal_strmatch` and consumes one BYTE.
//! A byte-only port makes `u/?.txt` leak `u/Ä.txt` under UTF-8 (and the
//! char-only predecessor made the C locale over-swallow); this pin covers
//! both faces so it discriminates on every platform — `LC_ALL` reaches the
//! spawned binary through the process environment and rubash's
//! `utf8_locale_active` accepts a UTF-8 name on unix and Windows alike.
//!
//! Expected values are byte-for-byte GNU bash 5.3.20(1) output recorded
//! on macOS arm64 with the fixture below.

use std::process::Command;

fn run(locale: &str, globignore: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue3605-mb-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(dir.join("u")).expect("mkdir fixture");
    for name in ["\u{C4}.txt", "ab.txt", "\u{C4}b.txt", "c.txt", "\u{E9}.txt"] {
        std::fs::write(dir.join("u").join(name), "").expect("touch fixture");
    }
    let script_path = dir.join("probe.sh");
    std::fs::write(
        &script_path,
        format!("GLOBIGNORE='{globignore}'\necho u/*.txt\n"),
    )
    .expect("write probe script");
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .arg(&script_path)
        .current_dir(&dir)
        .env("LC_ALL", locale)
        .env_remove("LC_CTYPE")
        .env_remove("LC_MESSAGES")
        .env_remove("LANG")
        .env_remove("BASH_ENV");
    let output = command.output().expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        output.status.success(),
        "probe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// UTF-8 locale: `?` swallows exactly one multibyte character, so a
/// single `?` hides `Ä.txt`/`é.txt`/`c.txt` but not the two-character
/// `ab.txt`/`Äb.txt`.
#[test]
fn globignore_question_matches_one_character_under_utf8() {
    assert_eq!(run("en_US.UTF-8", "u/?.txt"), "u/ab.txt u/\u{C4}b.txt\n");
}

/// UTF-8 locale: `??` swallows the two-character names, leaving the
/// single-character ones — the byte-wise compile over-swallows `Ä.txt`
/// only for `?` and under-swallows here.
#[test]
fn globignore_two_questions_match_two_characters_under_utf8() {
    assert_eq!(
        run("en_US.UTF-8", "u/??.txt"),
        "u/\u{C4}.txt u/c.txt u/\u{E9}.txt\n"
    );
}

/// UTF-8 locale: a bracket class holding a multibyte literal matches that
/// one character.
#[test]
fn globignore_multibyte_bracket_class_under_utf8() {
    assert_eq!(
        run("en_US.UTF-8", "u/[\u{C4}].txt"),
        "u/ab.txt u/\u{C4}b.txt u/c.txt u/\u{E9}.txt\n"
    );
}

/// C locale: matching stays byte-wise — `?` covers only the one-byte
/// name `c.txt`; `Ä`/`é` are two bytes and `ab` is two characters, so
/// only `c.txt` is swallowed. This is the face the previous char-only
/// engine got wrong.
#[test]
fn globignore_question_matches_one_byte_under_c_locale() {
    assert_eq!(
        run("C", "u/?.txt"),
        "u/ab.txt u/\u{C4}.txt u/\u{C4}b.txt u/\u{E9}.txt\n"
    );
}
