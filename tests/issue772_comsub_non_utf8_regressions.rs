//! Issue rubash#772 item 1: `$(cat FILE)` must treat file content as opaque
//! bytes, not as UTF-8 text.
//!
//! GNU `cat` reads a file operand as a byte stream (cat.c: safe_read ->
//! write) and `command_substitute` captures the child's pipe as a byte
//! buffer (subst.c:7143 -> read_comsub), so a non-UTF-8 file is ordinary
//! content. The word-level `cat FILE` command-substitution shortcut used
//! `std::fs::read_to_string`, so any invalid sequence failed the read and
//! the error arm reported a file that plainly exists as
//! `cat: 'FILE': No such file or directory` and returned an empty value
//! (`data=$(cat file)` silently became `""`). The shortcut now reads bytes
//! and rides the same `bytes_to_shell_text` carrier contract as every other
//! substitution capture.
//!
//! Expected bytes are verified against WSL GNU Bash 5.3.0
//! (`/usr/local/bin/bash`, script-file probes; `diff -r` of the result
//! files is byte-identical, stdout and stderr both empty).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue772-1-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue772 scratch dir");
    dir
}

/// Run `script` through rubash with `dir` as the sandboxed cwd.
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

fn read(dir: &Path, name: &str) -> Vec<u8> {
    std::fs::read(dir.join(name)).unwrap_or_else(|error| panic!("read {name}: {error}"))
}

/// Run `x=$(cat ...)` and return (value bytes, comsub status text, stderr, rc).
fn capture_cat(dir: &Path, operand: &str) -> (Vec<u8>, Vec<u8>, Vec<u8>, Option<i32>) {
    let script =
        format!("x=$(cat {operand}); rc=$?; printf '%s' \"$x\" > out; printf '%s' \"$rc\" > rc");
    let (stdout, stderr, code) = rubash(dir, &script);
    assert_eq!(stdout, b"", "script wrote unexpected stdout");
    (read(dir, "out"), read(dir, "rc"), stderr, code)
}

#[test]
fn invalid_ff_byte_is_content_not_a_missing_file() {
    let dir = scratch("ff");
    std::fs::write(dir.join("h"), [0xffu8]).unwrap();
    let (value, status, stderr, code) = capture_cat(&dir, "h");
    assert_eq!(stderr, b"", "no bogus 'No such file' diagnostic");
    assert_eq!(code, Some(0));
    assert_eq!(status, b"0");
    assert_eq!(value, vec![0xff]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn invalid_cd_byte_is_content() {
    let dir = scratch("cd");
    std::fs::write(dir.join("h"), [0xcdu8]).unwrap();
    let (value, status, stderr, code) = capture_cat(&dir, "h");
    assert_eq!(stderr, b"");
    assert_eq!(code, Some(0));
    assert_eq!(status, b"0");
    assert_eq!(value, vec![0xcd]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn concatenated_ascii_and_invalid_bytes() {
    // GNU: `$(cat g h)` with g=abc, h=0xff is `abc\xff`, len 4.
    let dir = scratch("concat");
    std::fs::write(dir.join("g"), b"abc").unwrap();
    std::fs::write(dir.join("h"), [0xffu8]).unwrap();
    let (value, status, stderr, code) = capture_cat(&dir, "g h");
    assert_eq!(stderr, b"");
    assert_eq!(code, Some(0));
    assert_eq!(status, b"0");
    assert_eq!(value, b"abc\xff");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn nul_free_binary_file_roundtrips_exactly() {
    let dir = scratch("bin");
    let bytes = b"A\xffB\xfeC\xcdD\x80E".to_vec();
    std::fs::write(dir.join("bin"), &bytes).unwrap();
    let (value, status, stderr, code) = capture_cat(&dir, "bin");
    assert_eq!(stderr, b"");
    assert_eq!(code, Some(0));
    assert_eq!(status, b"0");
    assert_eq!(value, bytes);
    // ${#x} counts each invalid byte as one character (GNU MB_STRLEN).
    let (stdout, stderr, code) = rubash(&dir, "x=$(cat bin); printf '%s' \"${#x}\" > len");
    assert_eq!(stdout, b"");
    assert_eq!(stderr, b"");
    assert_eq!(code, Some(0));
    assert_eq!(read(&dir, "len"), b"9");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn file_without_trailing_newline_is_preserved() {
    let dir = scratch("noeol");
    std::fs::write(dir.join("noeol"), b"noeol").unwrap();
    let (value, _, stderr, code) = capture_cat(&dir, "noeol");
    assert_eq!(stderr, b"");
    assert_eq!(code, Some(0));
    assert_eq!(value, b"noeol");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn trailing_newline_is_stripped_like_gnu() {
    // GNU subst.c command_substitute deletes trailing newlines; the file
    // content itself is untouched.
    let dir = scratch("withnl");
    std::fs::write(dir.join("withnl"), b"data\n").unwrap();
    let (value, _, stderr, code) = capture_cat(&dir, "withnl");
    assert_eq!(stderr, b"");
    assert_eq!(code, Some(0));
    assert_eq!(value, b"data");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn normal_utf8_file_is_an_unchanged_guard() {
    let dir = scratch("utf8");
    std::fs::write(dir.join("utf8"), "h\u{e9}llo\n".as_bytes()).unwrap();
    let (value, _, stderr, code) = capture_cat(&dir, "utf8");
    assert_eq!(stderr, b"");
    assert_eq!(code, Some(0));
    assert_eq!(value, "h\u{e9}llo".as_bytes());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn carrier_range_bytes_are_payload() {
    // 0x0c / 0x11 are parser carrier bytes, but they came OUT of `cat`, so
    // they are data and must reach the output boundary as those bytes.
    let dir = scratch("carrier");
    std::fs::write(dir.join("carrier"), b"a\x0cb\x11c").unwrap();
    let (value, _, stderr, code) = capture_cat(&dir, "carrier");
    assert_eq!(stderr, b"");
    assert_eq!(code, Some(0));
    assert_eq!(value, b"a\x0cb\x11c");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn length_of_invalid_byte_is_one() {
    let dir = scratch("len");
    std::fs::write(dir.join("h"), [0xffu8]).unwrap();
    let (stdout, stderr, code) = rubash(&dir, "x=$(cat h); printf '%s' \"${#x}\"");
    assert_eq!(stderr, b"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"1");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn redirect_and_stderr_redirect_forms_agree() {
    // `$(cat < h)` uses the real executor, and `2>/dev/null` disables the
    // word-level shortcut; both must agree with the fixed fast path.
    let dir = scratch("agree");
    std::fs::write(dir.join("h"), [0xffu8]).unwrap();
    let (stdout, stderr, code) = rubash(
        &dir,
        "a=$(cat < h); b=$(cat h 2>/dev/null); printf '%s' \"$a|$b\" > out",
    );
    assert_eq!(stdout, b"");
    assert_eq!(stderr, b"");
    assert_eq!(code, Some(0));
    assert_eq!(read(&dir, "out"), b"\xff|\xff");
    let _ = std::fs::remove_dir_all(&dir);
}
