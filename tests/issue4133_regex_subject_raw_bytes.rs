//! Issue unixwin/rubash#4133 (split from #4125): under a single-byte
//! locale (LC_ALL=C), the subject of `[[ =~ ]]` had its raw bytes
//! 0x80..=0xff UTF-8 re-encoded before matching and capture storage —
//! all 128 high bytes diverged from GNU:
//!
//! ```sh
//! s=$'a\x9cb'; [[ $s =~ ^(.*)$ ]]
//! # GNU stores BASH_REMATCH[1] as bytes 61 9c 62;
//! # rubash stored 61 c2 9c 62 (the byte-char spelling's UTF-8).
//! ```
//!
//! GNU contract: sh_regmatch regexec's the expanded operands as raw
//! bytes (execute_cmd.c:4096, via cond_expand_word mode 2). The
//! pattern side has the same provenance duty: a pattern carrying a
//! raw byte (via a variable) names the single byte, while a genuine
//! non-ASCII char names its UTF-8 byte sequence.
//!
//! Root cause: decode_regex_operand folds raw-byte marker pairs to
//! byte-chars (U+0000..=U+00FF), erasing provenance; the byte-mode
//! match then ran on the folded string's UTF-8 encoding, and the
//! pattern compile expanded folded byte-chars the same wrong way —
//! the two sides only agreed with each other, never with GNU.
//!
//! Expectations byte-verified against GNU bash 5.3.20 under LC_ALL=C
//! (script files, cloud oracle): captures below are compared as raw
//! file bytes.

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash4133-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

/// Run `script` under LC_ALL=C in a fresh tempdir; return the dir and
/// the run's stdout/stderr/rc so tests can read back capture files.
fn run_script(tag: &str, script: &str) -> (PathBuf, String, String, Option<i32>) {
    let dir = tempdir(tag);
    let file = dir.join("case.sh");
    std::fs::write(&file, script).expect("write script");
    let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&file)
        .current_dir(&dir)
        .env("LC_ALL", "C")
        .output()
        .expect("run rubash");
    (
        dir.clone(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

fn read(dir: &PathBuf, name: &str) -> Vec<u8> {
    std::fs::read(dir.join(name)).unwrap_or_default()
}

#[test]
fn issue4133_high_byte_scan_captures_stay_raw() {
    // Every byte 0x80..=0xff in the subject must come back as itself.
    let script = "for d in {128..255}; do\n\
                  \x20 printf -v h '%02x' \"$d\"\n\
                  \x20 printf -v z \"\\x$h\"\n\
                  \x20 s=\"a${z}b\"\n\
                  \x20 [[ $s =~ ^(.*)$ ]]\n\
                  \x20 printf '%s\\n' \"${BASH_REMATCH[1]}\" >> out.bin\n\
                  done\n";
    let (dir, _stdout, stderr, code) = run_script("scan", script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let bytes = read(&dir, "out.bin");
    let lines: Vec<&[u8]> = bytes.split(|b| *b == b'\n').collect();
    assert_eq!(lines.len(), 129, "128 captures plus trailing empty");
    for (i, line) in lines[..128].iter().enumerate() {
        let byte = 0x80u8 + i as u8;
        assert_eq!(*line, [0x61, byte, 0x62], "byte {byte:#04x} re-encoded");
    }
}

#[test]
fn issue4133_example_and_dot_capture() {
    let script = "s=$'a\\x9cb'\n\
                  [[ $s =~ ^(.*)$ ]]\n\
                  printf '%s' \"${BASH_REMATCH[1]}\" > out.bin\n\
                  [[ $s =~ ^a(.)b$ ]]\n\
                  printf '%s' \"${BASH_REMATCH[1]}\" > out2.bin\n";
    let (dir, _stdout, stderr, code) = run_script("example", script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(read(&dir, "out.bin"), [0x61, 0x9c, 0x62]);
    assert_eq!(read(&dir, "out2.bin"), [0x9c]);
}

#[test]
fn issue4133_genuine_utf8_char_keeps_its_bytes() {
    // A genuine é (bytes c3 a9) is two bytes to the C-locale engine,
    // before and after the fix; `.` walks bytes, `..` captures both.
    let script = "s=$'a\\xc3\\xa9b'\n\
                  [[ $s =~ ^(.*)$ ]]\n\
                  printf '%s' \"${BASH_REMATCH[1]}\" > out.bin\n\
                  [[ $s =~ ^a(..)b$ ]]\n\
                  printf '%s' \"${BASH_REMATCH[1]}\" > out2.bin\n";
    let (dir, _stdout, stderr, code) = run_script("genuine", script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(read(&dir, "out.bin"), [0x61, 0xc3, 0xa9, 0x62]);
    assert_eq!(read(&dir, "out2.bin"), [0xc3, 0xa9]);
}

#[test]
fn issue4133_pattern_raw_byte_still_matches_single_byte() {
    // The pattern side's provenance: a variable carrying byte 0x9c
    // must match the subject's single byte 0x9c (it only matched
    // before because both sides shared the wrong two-byte spelling).
    let script = "p=$'\\x9c'\n\
                  s=$'a\\x9cb'\n\
                  [[ $s =~ $p ]] && echo MATCH1\n\
                  [[ $s =~ ^a($p)b$ ]]\n\
                  printf '%s' \"${BASH_REMATCH[1]}\" > out.bin\n";
    let (dir, stdout, stderr, code) = run_script("pattern", script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "MATCH1\n");
    assert_eq!(read(&dir, "out.bin"), [0x9c]);
}

#[test]
fn issue4133_mixed_genuine_and_raw_subject() {
    // The rubash#3218 mix in one subject: genuine é beside raw 0x9c —
    // each keeps its own byte count.
    let script = "s=$'a\\xc3\\xa9\\x9cb'\n\
                  [[ $s =~ ^(.*)$ ]]\n\
                  printf '%s' \"${BASH_REMATCH[1]}\" > out.bin\n";
    let (dir, _stdout, stderr, code) = run_script("mixed", script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(read(&dir, "out.bin"), [0x61, 0xc3, 0xa9, 0x9c, 0x62]);
}
