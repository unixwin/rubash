//! Issue rubash#772 item 2: `${x:off:len}` must not fabricate carrier
//! characters for invalid bytes.
//!
//! GNU subst.c:9087-9092 (`parameter_brace_substring`) calls `mb_substring`
//! (subst.c:9022) in a multibyte locale. Its offset/length are character
//! counts and each step is one `ADVANCE_CHAR` (include/shmbutil.h:114-147):
//! a valid multibyte sequence advances by its byte length, and an invalid
//! byte (`mbrlen` == (size_t)-1 / -2) advances by one byte. Raw bytes >= 0x80
//! travel inside rubash words as U+E000 marker pairs, so the old
//! `value.chars()` slice cut a pair in half: `${x:0:1}` on `x=$'\xff'`
//! emitted `ee 80 80` (the UTF-8 encoding of the U+E000 introducer) and
//! `${x:1}` emitted `ee 84 80` (the U+E100 payload) instead of the raw byte
//! and the empty string.
//!
//! Expected bytes are verified against WSL GNU Bash 5.3.0
//! (`/usr/local/bin/bash`, script-file probes; `diff -r` of the result
//! files is byte-identical, stdout and stderr both empty).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue772-2-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue772 scratch dir");
    dir
}

fn rubash(dir: &Path, script: &str) -> (Vec<u8>, Vec<u8>, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash script");
    (output.stdout, output.stderr, output.status.code())
}

/// Run `script` (which must write its results into named files) and assert
/// the clean-process envelope.
fn run_clean(dir: &Path, script: &str) {
    let (stdout, stderr, code) = rubash(dir, script);
    assert_eq!(stdout, b"", "script must not write stdout");
    assert_eq!(stderr, b"", "script must not write stderr");
    assert_eq!(code, Some(0));
}

fn read(dir: &Path, name: &str) -> Vec<u8> {
    std::fs::read(dir.join(name)).unwrap_or_else(|error| panic!("read {name}: {error}"))
}

#[test]
fn single_invalid_byte_slices_to_the_raw_byte() {
    let dir = scratch("single");
    run_clean(
        &dir,
        r#"
x=$'\xff'
printf '%s' "${x:0:1}" > first
printf '%s' "${x:1}" > rest
printf '%s' "${x:0}" > whole
printf '%s' "${#x}" > len
"#,
    );
    assert_eq!(read(&dir, "first"), vec![0xff]);
    assert_eq!(read(&dir, "rest"), Vec::<u8>::new());
    assert_eq!(read(&dir, "whole"), vec![0xff]);
    assert_eq!(read(&dir, "len"), b"1");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn invalid_byte_between_ascii_is_one_character() {
    let dir = scratch("middle");
    run_clean(
        &dir,
        r#"
y=$'a\xffb'
printf '%s' "${y:0:1}" > p01
printf '%s' "${y:1:1}" > p11
printf '%s' "${y:1}" > p1
printf '%s' "${y:2}" > p2
printf '%s' "${y:2:1}" > p21
printf '%s' "${y: -1}" > pneg
printf '%s' "${#y}" > len
"#,
    );
    assert_eq!(read(&dir, "p01"), b"a");
    assert_eq!(read(&dir, "p11"), vec![0xff]);
    assert_eq!(read(&dir, "p1"), b"\xffb");
    assert_eq!(read(&dir, "p2"), b"b");
    assert_eq!(read(&dir, "p21"), b"b");
    assert_eq!(read(&dir, "pneg"), b"b");
    assert_eq!(read(&dir, "len"), b"3");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn consecutive_invalid_bytes_are_separate_characters() {
    let dir = scratch("consecutive");
    run_clean(
        &dir,
        r#"
v=$'\xff\xfe'
printf '%s' "${v:0:2}" > both
printf '%s' "${v:1:1}" > second
printf '%s' "${#v}" > len
"#,
    );
    assert_eq!(read(&dir, "both"), vec![0xff, 0xfe]);
    assert_eq!(read(&dir, "second"), vec![0xfe]);
    assert_eq!(read(&dir, "len"), b"2");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn utf8_guard_still_slices_by_character() {
    // A normal UTF-8 value must be untouched: é is one character.
    let dir = scratch("utf8");
    run_clean(
        &dir,
        r#"
z=$'a\u00e9b'
printf '%s' "${z:1:1}" > z11
printf '%s' "${z:0:2}" > z02
"#,
    );
    assert_eq!(read(&dir, "z11"), vec![0xc3, 0xa9]);
    assert_eq!(read(&dir, "z02"), b"a\xc3\xa9");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn multibyte_guard_counts_one_character_per_sequence() {
    // € is a 3-byte sequence and one character in the UTF-8 locale.
    let dir = scratch("euro");
    run_clean(
        &dir,
        r#"
e=$'\xe2\x82\xacA'
printf '%s' "${e:0:1}" > e01
printf '%s' "${e:0:2}" > e02
"#,
    );
    assert_eq!(read(&dir, "e01"), b"\xe2\x82\xac");
    assert_eq!(read(&dir, "e02"), b"\xe2\x82\xacA");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn slices_never_emit_carrier_marker_bytes() {
    // Regression pin: the old multibyte path emitted the UTF-8 encoding of
    // the U+E000 introducer (`ee 80 80`) / U+E100 payload (`ee 84 80`).
    let dir = scratch("nomarkers");
    run_clean(
        &dir,
        r#"
x=$'\xff'
printf '%s' "${x:0:1}" > first
printf '%s' "${x:1}" > rest
"#,
    );
    let first = read(&dir, "first");
    assert_ne!(first, b"\xee\x80\x80");
    assert_ne!(first, b"\xee\x84\x80");
    assert_eq!(first, vec![0xff]);
    assert_eq!(read(&dir, "rest"), Vec::<u8>::new());
    let _ = std::fs::remove_dir_all(&dir);
}
