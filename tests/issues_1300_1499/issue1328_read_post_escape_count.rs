//! Issue rubash#1328: `read -n`/`-N` without `-r` count POST-ESCAPE
//! characters (GNU builtins/read.def processes `\x` inline while
//! counting): an escape pair costs one unit, a backslash-newline
//! continuation costs zero, and a quoted delimiter does not end the
//! record. `-r` keeps raw counting.
//!
//! Expected outputs verified against GNU bash 5.3.20.

use std::io::Write;
use std::process::{Command, Stdio};

fn read_with_stdin(args: &str, bytes: &[u8]) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(format!("IFS= read {args} x; printf '<%s>' \"$x\""))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(bytes)
        .expect("feed stdin");
    let output = child.wait_with_output().expect("wait");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn read_n_counts_escape_pair_as_one() {
    // Stream: a \ b — the pair is one stored character.
    assert_eq!(read_with_stdin("-N 2", b"a\\b"), "<ab>");
    assert_eq!(read_with_stdin("-n 2", b"a\\bc"), "<ab>");
}

#[test]
fn read_n_continuation_costs_zero() {
    // Stream: a \ <LF> b c — continuation appends nothing.
    assert_eq!(read_with_stdin("-N 2", b"a\\\nbc"), "<ab>");
    assert_eq!(read_with_stdin("-N 3", b"a\\\nb"), "<ab>");
}

#[test]
fn read_continuation_does_not_end_record() {
    assert_eq!(read_with_stdin("-n 5", b"a\\\nzz\n"), "<azz>");
    assert_eq!(read_with_stdin("", b"a\\\nzz\n"), "<azz>");
}

#[test]
fn read_r_keeps_raw_counting() {
    assert_eq!(read_with_stdin("-rN 3", b"a\\b"), "<a\\b>");
    assert_eq!(read_with_stdin("-rn 2", b"a\\bc"), "<a\\>");
}
