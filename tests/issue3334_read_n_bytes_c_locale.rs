//! rubash#3334 — `read -n`/`-N` counts characters instead of bytes in a
//! single-byte locale.
//!
//! GNU builtins/read.def:874 only calls read_mbchar when `mb_cur_max > 1`,
//! so with LC_ALL=C every byte is one unit (`nr++` per byte, read.def:880-
//! 886): `read -n 1` over `é` (c3 a9) takes just the lead byte, and
//! `read -n 2` takes both bytes of `é`. rubash's StdinCharDecoder grouped
//! a valid UTF-8 sequence into one Char unit even in a single-byte locale,
//! so `-n 1` took the whole `é` and `-n 2` took `é` + `x`.
//!
//! GNU bash 5.3.20 oracle expectations, from the issue's regression seed
//! (verified: GNU 4/4 PASS; rubash b04c82c7 3 FAIL + ASCII PASS). The
//! piped-stdin `-c` shape is the inherited-stdin serve path
//! (read_inherited_process_stdin); the sequential-read case also pins the
//! stream offset (after `-n 1` takes [c3], the next read must see [a9]).
//! Controls lock the ASCII face, the plain-read face, and the C.utf8 face
//! (character counting is correct there).

use std::io::Write;
use std::process::{Command, Stdio};

fn run_c_stdin(script: &str, locale: &str, stdin: &[u8]) -> (Vec<u8>, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env("LC_ALL", locale)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(stdin)
        .expect("write stdin");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait rubash");
    (
        output.stdout,
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn read_n1_takes_first_byte_in_c_locale() {
    let (stdout, stderr, code) = run_c_stdin("read -n 1 x; printf %s \"$x\"", "C", b"\xc3\xa9x\n");
    assert_eq!(code, Some(0), "stderr: {stderr:?}");
    assert_eq!(stderr, "", "stderr: {stderr:?}");
    assert_eq!(stdout, vec![0xc3]);
}

#[test]
fn read_n2_takes_two_bytes_in_c_locale() {
    let (stdout, stderr, code) = run_c_stdin("read -n 2 x; printf %s \"$x\"", "C", b"\xc3\xa9x\n");
    assert_eq!(code, Some(0), "stderr: {stderr:?}");
    assert_eq!(stderr, "", "stderr: {stderr:?}");
    assert_eq!(stdout, vec![0xc3, 0xa9]);
}

#[test]
fn read_upper_n1_takes_first_byte_in_c_locale() {
    let (stdout, stderr, code) = run_c_stdin("read -N 1 x; printf %s \"$x\"", "C", b"\xc3\xa9x\n");
    assert_eq!(code, Some(0), "stderr: {stderr:?}");
    assert_eq!(stderr, "", "stderr: {stderr:?}");
    assert_eq!(stdout, vec![0xc3]);
}

#[test]
fn read_upper_n2_takes_two_bytes_in_c_locale() {
    let (stdout, stderr, code) = run_c_stdin("read -N 2 x; printf %s \"$x\"", "C", b"\xc3\xa9x\n");
    assert_eq!(code, Some(0), "stderr: {stderr:?}");
    assert_eq!(stderr, "", "stderr: {stderr:?}");
    assert_eq!(stdout, vec![0xc3, 0xa9]);
}

#[test]
fn read_n_sequential_reads_consume_byte_by_byte_in_c_locale() {
    let (stdout, stderr, code) = run_c_stdin(
        "read -n 1 a; read -n 1 b; read -n 1 c; printf '%s|%s|%s' \"$a\" \"$b\" \"$c\"",
        "C",
        b"\xc3\xa9x\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr:?}");
    assert_eq!(stderr, "", "stderr: {stderr:?}");
    assert_eq!(stdout, vec![0xc3, b'|', 0xa9, b'|', 0x78]);
}

#[test]
fn read_n1_ascii_control_in_c_locale() {
    let (stdout, stderr, code) = run_c_stdin("read -n 1 x; printf %s \"$x\"", "C", b"ab\n");
    assert_eq!(code, Some(0), "stderr: {stderr:?}");
    assert_eq!(stderr, "", "stderr: {stderr:?}");
    assert_eq!(stdout, vec![0x61]);
}

#[test]
fn read_plain_no_limit_control_in_c_locale() {
    let (stdout, stderr, code) = run_c_stdin("read x; printf %s \"$x\"", "C", b"\xc3\xa9x\n");
    assert_eq!(code, Some(0), "stderr: {stderr:?}");
    assert_eq!(stderr, "", "stderr: {stderr:?}");
    assert_eq!(stdout, vec![0xc3, 0xa9, 0x78]);
}

#[test]
fn read_n1_char_control_in_utf8_locale() {
    // Control: in a multibyte locale the whole character is one unit.
    let (stdout, stderr, code) =
        run_c_stdin("read -n 1 x; printf %s \"$x\"", "C.UTF-8", b"\xc3\xa9x\n");
    assert_eq!(code, Some(0), "stderr: {stderr:?}");
    assert_eq!(stderr, "", "stderr: {stderr:?}");
    assert_eq!(stdout, vec![0xc3, 0xa9]);
}

#[test]
fn read_n2_char_control_in_utf8_locale() {
    let (stdout, stderr, code) =
        run_c_stdin("read -n 2 x; printf %s \"$x\"", "C.UTF-8", b"\xc3\xa9x\n");
    assert_eq!(code, Some(0), "stderr: {stderr:?}");
    assert_eq!(stderr, "", "stderr: {stderr:?}");
    assert_eq!(stdout, vec![0xc3, 0xa9, 0x78]);
}
