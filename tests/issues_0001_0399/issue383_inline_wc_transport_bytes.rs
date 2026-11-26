// niubash#383: the inline `wc` pipeline fast path measured TRANSPORT text,
// not the payload. Carrier bytes (0x1B, 0x0C, ...) travel between the lexer
// and the executor as U+E000+U+E0xx raw-byte marker pairs (locale.rs:352), so
// `input.as_bytes().len()` counts six bytes for a one-byte ESC and reports
// `printf '\033' | wc -c` as 6. The external WinuxCmd wc is correct; the
// sibling unixwin/WinuxCmd#383 is a `date` issue and is NOT this bug — this
// one is labelled `Winuxcmd-bug` only because the reporter watched the
// bundled coreutils.
//
// The payload from the issue report is 10 bytes, not 9: ESC [ 3 1 m X ESC
// [ 0 m. The "expected 9" in the report title was a miscount.
//
// Byte and word counts must both run over the payload bytes the external wc
// would receive (shell_text_to_raw_bytes): there, a carrier-encoded
// whitespace byte (0x0C) still delimits words; payload itself must never be
// fed to decode_to_visible_text, because a payload control byte maps to
// CTLESC and trips that decoder's postcondition (panic in debug builds).

use std::process::Command;

fn run(cmd: &str) -> (String, i32) {
    let exe = env!("CARGO_BIN_EXE_rubash");
    let out = Command::new(exe)
        .arg("-c")
        .arg(cmd)
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

/// The reporter's rule table: N control bytes count N, not 6N.
#[test]
fn byte_count_control_byte_sweep() {
    for n in 0..=3usize {
        let escs = "\\033".repeat(n);
        let (out, rc) = run(&format!("printf '{escs}' | wc -c"));
        assert_eq!(
            (out.as_str(), rc),
            (format!("{n}\n").as_str(), 0),
            "{n} ESC byte(s) must count {n}"
        );
    }
}

/// `ESC [ 3 1 m X ESC [ 0 m` is 10 bytes (the report's "9" was wrong).
#[test]
fn byte_count_esc_sequence() {
    let (out, rc) = run("printf '\\033[31mX\\033[0m' | wc -c");
    assert_eq!((out.as_str(), rc), ("10\n", 0));
}

/// The exact minimal repro from the issue: `\x1b[0m` is 4 bytes, not 9.
#[test]
fn byte_count_issue_minimal_repro() {
    let (out, rc) = run("printf '\\033[0m' | wc -c");
    assert_eq!((out.as_str(), rc), ("4\n", 0));
}

/// The flagless form prints lines, words, and bytes; the byte field decodes.
#[test]
fn default_form_decodes_bytes() {
    let (out, rc) = run("printf '\\033' | wc");
    assert_eq!((out.as_str(), rc), ("      0       1       1\n", 0));
}

/// The `$( ... | wc -c )` fast path (command_substitution_values.rs) has the
/// same carrier leak and must decode too.
#[test]
fn command_substitution_byte_count() {
    let (out, rc) = run("echo \"$(printf '\\033' | wc -c)\"");
    assert_eq!((out.as_str(), rc), ("1\n", 0));
}

/// Form feed (0x0C) is carrier-encoded AND whitespace: decoding before the
/// word split keeps GNU's 2-word count (`a\014b`).
#[test]
fn word_count_decodes_carrier_whitespace() {
    let (out, rc) = run("printf 'a\\014b' | wc -w");
    assert_eq!((out.as_str(), rc), ("2\n", 0));
}

/// `read` review R1 on PR #695: `-w` used to pipe payload through
/// `decode_to_visible_text`, whose postcondition panics when a payload
/// control byte (`0x11` here) decodes to CTLESC. Word counting now runs over
/// `shell_text_to_raw_bytes` and must neither panic (debug) nor miscount.
#[test]
fn word_count_control_payload_no_panic() {
    let (out, rc) = run("printf '\\021' | wc -w");
    assert_eq!((out.as_str(), rc), ("1\n", 0));
}

/// Same payload through the flagless form: the words field must agree.
#[test]
fn default_form_word_field_no_panic() {
    let (out, rc) = run("printf '\\021' | wc");
    assert_eq!((out.as_str(), rc), ("      0       1       1\n", 0));
}

/// Tab, form feed, and newline are all word delimiters over payload bytes,
/// while a carrier like ESC stays inside a word.
#[test]
fn word_count_payload_whitespace_mix() {
    let (out, rc) = run("printf 'a\\tb\\014c\\nd' | wc -w");
    assert_eq!((out.as_str(), rc), ("4\n", 0));
}

/// `-l` counts `\n`; 0x0A is not a carrier so it needs no decode and must not
/// regress.
#[test]
fn line_count_is_unchanged() {
    let (out, rc) = run("printf 'a\\033\\nb' | wc -l");
    assert_eq!((out.as_str(), rc), ("1\n", 0));
}

/// `-m` and clustered flags fall back to the external wc and stay correct;
/// pin them so a future fast-path arm cannot silently take them over wrong.
#[test]
fn external_fallback_arms_stay_correct() {
    let (chars, rc) = run("printf '\\033' | wc -m");
    assert_eq!((chars.as_str(), rc), ("1\n", 0), "-m is character count");

    let (clustered, rc) = run("printf 'a\\033\\nb' | wc -lc");
    assert_eq!(
        (clustered.as_str(), rc),
        ("      1       4\n", 0),
        "-lc is not a single-flag fast path"
    );
}

/// ASCII payloads are unaffected by the decode step.
#[test]
fn ascii_payload_unchanged() {
    let (out, rc) = run("printf 'abc' | wc -c");
    assert_eq!((out.as_str(), rc), ("3\n", 0));
}
