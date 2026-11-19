//! rubash#3943 — a multibyte `read -d` delimiter matches only the FIRST
//! byte of its `-d` word's encoding (builtins/read.def:385 + 834:
//! delimiter eth U+00F0 arrives as bytes C3 B0 and keys on 0xC3, never
//! on the codepoint 0xF0). Rubash additionally stopped on raw input
//! bytes equal to the delimiter character's codepoint, on every serve:
//! the inherited-stdin unit walker compared `delimiter as u8`, and the
//! text-domain single-byte form encoded the codepoint — so a lone
//! 0xE9/0xF0/0xFF byte or a valid emoji (lead byte 0xF0 vs delimiter ð)
//! cut the record where GNU reads straight through.
//!
//! GNU bash 5.3.20 oracle expectations, measured with script files
//! under LC_ALL=C.UTF-8; the controls pin the first-byte matches both
//! shells agree on.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn run_script_locale(
    script: &str,
    locale: &str,
    stdin_bytes: Option<&[u8]>,
) -> (String, Option<i32>) {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir: PathBuf = std::env::temp_dir().join(format!("rubash-3943-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .arg(path.to_str().expect("utf8 path"))
        .env("LC_ALL", locale)
        .env("TMPDIR", &dir);
    let stdin_data;
    match stdin_bytes {
        Some(bytes) => {
            use std::io::Write;
            stdin_data = bytes.to_vec();
            command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let mut child = command.spawn().expect("spawn rubash");
            child
                .stdin
                .take()
                .expect("stdin pipe")
                .write_all(&stdin_data)
                .expect("write stdin");
            let output = child.wait_with_output().expect("run rubash");
            let _ = std::fs::remove_dir_all(&dir);
            return (
                String::from_utf8_lossy(&output.stdout).into_owned(),
                output.status.code(),
            );
        }
        None => {
            command.stdin(Stdio::null());
        }
    }
    let output = command.output().expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.code(),
    )
}

fn hex_pipeline(script: &str) -> String {
    const HEX: &str = "hex() { printf '%s' \"$1\" | od -An -tx1 | tr -d ' \\n'; }\n";
    let (stdout, code) = run_script_locale(&format!("{HEX}{script}"), "C.UTF-8", None);
    assert_eq!(code, Some(0));
    stdout
}

#[test]
fn eth_delimiter_reads_through_emoji_on_stdin() {
    // Delimiter ð (U+00F0, word bytes C3 B0) vs an emoji whose UTF-8
    // encoding STARTS with 0xF0: GNU reads through it (the operative
    // byte is 0xC3), rubash stopped at the emoji.
    let script = "read -d $'\\303\\260' -n 5 x; rc=$?; printf 'x=[%s] rc=%d\\n' \"$x\" \"$rc\"\n";
    let (stdout, code) = run_script_locale(script, "C.UTF-8", Some(b"ab\xf0\x9f\x98\x80cd"));
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "x=[ab😀cd] rc=0\n");
}

#[test]
fn ydieresis_delimiter_reads_through_lone_ff_byte() {
    let script = "printf 'ab\\377c' > \"$TMPDIR/f\"; exec 3< \"$TMPDIR/f\"; read -d $'\\303\\277' -n 5 -u 3 x; \
                  rc=$?; echo \"rc=$rc $(printf '%s' \"$x\" | od -An -tx1 | tr -d ' \\n')\"\n";
    let stdout = hex_pipeline(script);
    // GNU: reads through the raw 0xFF byte; rc 1 because -n 5 counts only
    // 4 units (a, b, the one-byte invalid 0xFF unit, c) before EOF.
    assert_eq!(stdout, "rc=1 6162ff63\n");
}

#[test]
fn u0080_delimiter_reads_through_lone_80_byte() {
    let script = "printf 'ab\\200cd' > \"$TMPDIR/f\"; exec 3< \"$TMPDIR/f\"; read -d $'\\302\\200' -n 5 -u 3 x; \
                  rc=$?; echo \"rc=$rc $(printf '%s' \"$x\" | od -An -tx1 | tr -d ' \\n')\"\n";
    let stdout = hex_pipeline(script);
    assert_eq!(stdout, "rc=0 6162806364\n");
}

#[test]
fn eacute_delimiter_reads_through_lone_e9_byte() {
    let script = "printf 'ab\\351cd' > \"$TMPDIR/f\"; exec 3< \"$TMPDIR/f\"; read -d $'\\303\\251' -n 5 -u 3 x; \
                  rc=$?; echo \"rc=$rc $(printf '%s' \"$x\" | od -An -tx1 | tr -d ' \\n')\"\n";
    let stdout = hex_pipeline(script);
    // GNU: the invalid lead byte does not count toward -n, so 4 of 5
    // units before EOF leaves rc 1 (verified against the oracle).
    assert_eq!(stdout, "rc=1 6162e96364\n");
}

#[test]
fn eth_delimiter_reads_through_lone_f0_byte() {
    let script = "printf 'ab\\360x' > \"$TMPDIR/f\"; exec 3< \"$TMPDIR/f\"; read -d $'\\303\\260' -n 5 -u 3 x; \
                  rc=$?; echo \"rc=$rc $(printf '%s' \"$x\" | od -An -tx1 | tr -d ' \\n')\"\n";
    let stdout = hex_pipeline(script);
    assert_eq!(stdout, "rc=1 6162f078\n");
}

#[test]
fn eacute_stops_on_first_byte_c3() {
    // Control: a lone 0xC3 byte IS the delimiter's first byte — both
    // shells stop there.
    let script = "printf 'ab\\303(cd' > \"$TMPDIR/f\"; exec 3< \"$TMPDIR/f\"; read -d $'\\303\\251' -n 5 -u 3 x; \
                  echo \"x=[$x] rc=$?\"\n";
    let stdout = hex_pipeline(script);
    assert_eq!(stdout, "x=[ab] rc=0\n");
}

#[test]
fn euro_delimiter_stops_on_first_byte_e2() {
    let script = "printf 'ab\\342' > \"$TMPDIR/f\"; exec 3< \"$TMPDIR/f\"; read -d $'\\342\\202\\254' -n 5 -u 3 x; \
                  echo \"x=[$x] rc=$?\"\n";
    let stdout = hex_pipeline(script);
    assert_eq!(stdout, "x=[ab] rc=0\n");
}

#[test]
fn proper_eacute_stops_both_faces() {
    let script = "printf 'ab\\303\\251cd' > \"$TMPDIR/f\"; exec 3< \"$TMPDIR/f\"; read -d $'\\303\\251' -n 5 -u 3 x; \
                  echo \"x=[$x] rc=$?\"\n";
    let stdout = hex_pipeline(script);
    assert_eq!(stdout, "x=[ab] rc=0\n");
}
