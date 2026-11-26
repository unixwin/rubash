//! Issues rubash#349/#350 — extglob in substitution patterns and pathname
//! expansion.
//!
//! #349: `${v/@(abc)/X}` left the value unchanged — the pattern-substitution
//! literal fast path's admission (pattern_contains_glob) checked only the
//! basic glob set (`*?[\\`), so an extglob operator group (`@(`, `!(`, `+(`,
//! `*(`, `?(`) fell to plain string replacement and never matched. GNU
//! pat_subst (subst.c:9238) routes every pattern through match_pattern /
//! match_upattern, which honors FNM_EXTMATCH when the extglob option is on;
//! with it off the group characters are ordinary literals.
//!
//! #350: `echo @('a b')` returned the literal pattern — the per-word
//! pathname suppression (raw_word_suppresses_pathname_expansion) treated
//! ANY quote in the raw as "quoted word, no recorded pattern => no glob".
//! GNU quoting is per-character (glob.c udequote_pathname): an UNQUOTED
//! extglob opener (read_token_word's PATTERN_CHAR arm, parse.y:5466-5489)
//! keeps the whole word a live pattern while the quoted characters inside
//! the group match literally — `@('a b')` matches the file `a b`.
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-28).

use std::path::Path;
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

/// #349: extglob operator groups substitute in the patsub position.
#[test]
fn extglob_patsub_substitutes() {
    let (stdout, stderr, code) = rubash(
        "shopt -s extglob\nv=abcabc\necho \"${v/@(abc)/X}\"\necho \"${v//@(abc)/X}\"\necho \"${v/!(a)/Z}\"\necho \"${v/+(ab)/Y}\"",
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    // GNU probes: `+(ab)` matches one repetition (Ycabc).
    assert_eq!(stdout, "Xabc\nXX\nZ\nYcabc\n");
}

/// #349 guard: with extglob OFF the operator characters are literals — the
/// compiled matcher answers that too (GNU: no FNM_EXTMATCH).
#[test]
fn extglob_patsub_literal_without_shopt() {
    let (stdout, _, code) = rubash("v=abc\necho \"${v/@(abc)/X}\"\necho \"${v/@(a)/X}\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "abc\nabc\n");
}

/// #349 guard: basic glob patsub behavior is unchanged.
#[test]
fn basic_glob_patsub_unchanged() {
    let (stdout, _, code) =
        rubash("v=abcab\necho \"${v/a*/X}\"\necho \"${v//b/Y}\"\necho \"${v/#a/Z}\"");
    assert_eq!(code, Some(0));
    // GNU: `a*` consumes to end of value (leftmost match, `*` spans).
    assert_eq!(stdout, "X\naYcaY\nZbcab\n");
}

/// #350: quoted characters inside an extglob group are literal pattern
/// text — `@('a b')` matches the file named `a b`.
#[test]
fn quoted_extglob_group_matches_filename_with_space() {
    let dir = std::env::temp_dir().join(format!("rubash-extglob350-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a b"), b"").unwrap();
    std::fs::write(dir.join("a"), b"").unwrap();
    let dir_display = dir.to_string_lossy().replace('\\', "/");
    let (stdout, stderr, code) = rubash(&format!(
        "cd {dir_display:?}\nshopt -s extglob\necho @('a b')"
    ));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "a b\n");
}

/// #350 guard: a FULLY quoted extglob pattern is a literal word.
#[test]
fn fully_quoted_extglob_pattern_stays_literal() {
    let dir = std::env::temp_dir().join(format!("rubash-extglob350b-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a"), b"").unwrap();
    let dir_display = dir.to_string_lossy().replace('\\', "/");
    let (stdout, _, code) = rubash(&format!(
        "cd {dir_display:?}\nshopt -s extglob\necho '@(a)'"
    ));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "@(a)\n");
    assert!(Path::new(&dir).exists() == false || true);
}

/// #350: unquoted extglob pathname expansion keeps working (the regression
/// guard for the suppression change).
#[test]
fn unquoted_extglob_pathname_still_expands() {
    let dir = std::env::temp_dir().join(format!("rubash-extglob350c-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ab"), b"").unwrap();
    std::fs::write(dir.join("ax"), b"").unwrap();
    let dir_display = dir.to_string_lossy().replace('\\', "/");
    let (stdout, stderr, code) =
        rubash(&format!("cd {dir_display:?}\nshopt -s extglob\necho @(a?)"));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("ab") && stdout.contains("ax"),
        "stdout: {stdout}"
    );
}
