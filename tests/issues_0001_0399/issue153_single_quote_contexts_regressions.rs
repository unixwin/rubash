//! rubash#153 — The `'a'\''b'` splice form and ERE intervals in
//! non-plain-word positions.
//!
//! GNU parse.y read_token_word: the close-quote / escaped-quote /
//! open-quote splice `'a'\''b'` assembles the literal word a'b in
//! EVERY word position — as a case subject AND pattern, as a
//! herestring, and as a pipeline word — not just in plain argument
//! position. The adjacent form pins the unquoted-ERE interval
//! operator: `^ab{2}c$` must match `abbc` (the braces are regex
//! syntax, not a brace expansion).
//!
//! Expectations byte-verified against GNU bash 5.3.20 with the filed
//! probe payloads verbatim (`-c` strings).

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

#[test]
fn quote_splice_in_case_subject_and_pattern() {
    let (stdout, _, code) = rubash("case 'a'\\''b' in 'a'\\''b') echo hit;; *) echo miss;; esac");
    assert_eq!(stdout, "hit\n");
    assert_eq!(code, Some(0));
}

#[test]
fn quote_splice_in_herestring() {
    let (stdout, _, code) = rubash("cat <<< 'a'\\''b'");
    assert_eq!(stdout, "a'b\n");
    assert_eq!(code, Some(0));
}

#[test]
fn quote_splice_in_pipeline_word() {
    let (stdout, _, code) = rubash("echo 'a'\\''b' | cat");
    assert_eq!(stdout, "a'b\n");
    assert_eq!(code, Some(0));
}

#[test]
fn ere_interval_in_regex_match() {
    let (stdout, _, code) = rubash("[[ abbc =~ ^ab{2}c$ ]] && echo ere-m || echo ere-n");
    assert_eq!(stdout, "ere-m\n");
    assert_eq!(code, Some(0));
}
