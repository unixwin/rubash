//! niubash#277: `read` must store 0x1b/0x0c input bytes in the same domain
//! as `$'..'` words.
//!
//! The assignment/ANSI-C transport stores carrier bytes (0x0c, 0x11, 0x13,
//! 0x14..=0x1f — QUOTED_WORD_PREFIX is 0x1b) as the E000 raw-byte marker
//! pair, and so do the file/coproc read transports via bytes_to_shell_text.
//! The byte transports (inherited process stdin, the FUNCTION_STDIN mirror)
//! used to store them as literal chars, so the value collided with the
//! lexer's marker system: `declare -p` rendered the ESC invisible, pattern
//! comparison against `$'\x1b[A'` never matched (direction-key parsing in
//! TUI scripts), and a value that crossed the assignment boundary again came
//! out as the raw marker bytes ee 80 80.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_stdin(args: &[&str], stdin: &[u8]) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(stdin)
        .expect("write stdin");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

const ESC_FF_LINE: &[u8] = b"\x1b[A\x0cX\n";

/// Byte fidelity: `printf '%s' "$v"` re-emits exactly the input bytes
/// (guard — the output boundary decoded both domains, but pins the
/// contract; the exact equality also proves no ee 80 80 marker bytes
/// leaked into the record).
#[test]
fn read_pipe_bytes_roundtrip() {
    let (stdout, stderr, rc) = run_stdin(
        &["-c", "read v; printf '%s' \"$v\" | od -An -tx1"],
        ESC_FF_LINE,
    );
    assert_eq!(stdout, " 1b 5b 41 0c 58\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}

/// The read value must equal the `$'\x1b[A\x0cX'` word — the direction-key
/// dispatch shape (`case $key in $'\x1b[A') ...`) depends on it.
#[test]
fn read_value_matches_ansi_c_word() {
    let script = "read v\nv2=$'\\x1b[A\\x0cX'\n[[ \"$v\" == \"$v2\" ]] && echo MATCH || echo NOMATCH\ncase \"$v\" in $'\\x1b[A\\x0cX') echo CASE_MATCH;; *) echo CASE_NOMATCH;; esac\n";
    let (stdout, stderr, rc) = run_stdin(&["-c", script], ESC_FF_LINE);
    assert_eq!(stdout, "MATCH\nCASE_MATCH\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}

/// `declare -p` renders the ESC (`\E`) instead of dropping it as an
/// internal marker (GNU: `declare -- v=$'\E[A\fX'`).
#[test]
fn declare_p_renders_esc() {
    let (stdout, stderr, rc) = run_stdin(&["-c", "read v; declare -p v"], ESC_FF_LINE);
    assert_eq!(stdout, "declare -- v=$'\\E[A\\fX'\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}

/// No marker bytes may leak through any consumer of the record: the
/// sentinel U+E000 encodes as ee 80 80 — its absence is part of the
/// contract, alongside the byte fidelity above.
#[test]
fn no_marker_bytes_in_any_output() {
    const MARKER_SENTINEL: &str = "\u{e000}";
    let script = "read v\nprintf '%s\\n' \"$v\"\ndeclare -p v\nset -x\n: \"$v\"\nset +x\n";
    let (stdout, stderr, rc) = run_stdin(&["-c", script], ESC_FF_LINE);
    assert!(
        !stdout.contains(MARKER_SENTINEL),
        "stdout leaked marker bytes: {stdout:?}"
    );
    assert!(
        !stderr.contains(MARKER_SENTINEL),
        "stderr leaked marker bytes: {stderr:?}"
    );
    // And the raw UTF-8 form (ee 80 80) is the same assertion at the byte
    // level, on the exact output the shell emitted.
    assert_eq!(rc, Some(0));
    let raw: Vec<u8> = stdout.bytes().collect();
    assert!(
        !raw.windows(3).any(|w| w == [0xee, 0x80, 0x80]),
        "stdout leaked ee 80 80 sentinel bytes"
    );
    let raw_err: Vec<u8> = stderr.bytes().collect();
    assert!(
        !raw_err.windows(3).any(|w| w == [0xee, 0x80, 0x80]),
        "stderr leaked ee 80 80 sentinel bytes"
    );
}

/// Length counts the ESC: `${#v}` is 5 after `read` on 1b 5b 41 0c 58.
#[test]
fn length_counts_esc_byte() {
    let (stdout, stderr, rc) = run_stdin(&["-c", "read v; echo \"len=${#v}\""], ESC_FF_LINE);
    assert_eq!(stdout, "len=5\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}

/// A form-feed byte is data, not IFS whitespace: default IFS is " \t\n",
/// so `read -a` keeps the record whole (GNU: 1 element).
#[test]
fn form_feed_does_not_split_array_read() {
    let (stdout, stderr, rc) = run_stdin(
        &["-c", "read -a A; echo \"n=${#A[@]} first=${A[0]}\""],
        b"x\x0cy\n",
    );
    assert_eq!(stdout, "n=1 first=x\x0cy\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}

/// `-d $'\x1b'` delimits on the ESC byte across transports: the mirror
/// (pipeline stage) stores raw bytes while the served record is tagged —
/// both forms must terminate the record and strip the delimiter.
#[test]
fn esc_delimiter_works_through_stage_mirror() {
    let script =
        "g() { read -d $'\\x1b' v; printf '%s' \"$v\" | od -An -tx1; }; printf 'a\\x1bb\\n' | g\n";
    let (stdout, stderr, rc) = run_stdin(&["-c", script], b"");
    assert_eq!(stdout, " 61\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}
