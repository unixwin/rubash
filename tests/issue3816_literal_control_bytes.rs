//! rubash#3816: literal control bytes in quoted words are corrupted —
//! 10 of the 32 CC constants (CC11, CC13, CC14, CC16, CC17, CC18, CC1A,
//! CC1C, CC1D, CC1F) come back as shell metacharacters or empty.
//!
//! Root cause (mechanism): a literal C0 byte inside a quoted word enters
//! transport text bare, where it is byte-identical to one of the C0
//! carrier markers (markers.rs: CTLESC 0x11, PARAM_NAME_END_MARKER 0x13,
//! DATA_BACKSLASH 0x14, PROTECTED_ESCAPED_SQUOTE 0x16, DATA_SQUOTE 0x17,
//! DATA_DQUOTE 0x18, DATA_BACKTICK 0x1a, IFS_GLUE 0x1c,
//! STORAGE_WORD_PREFIX 0x1d, DATA_DOLLAR 0x1f). Downstream decoders then
//! claim it as protocol: the assignment boundary drops CTLESC pairs and
//! IFS_GLUE (assignment_expansion.rs dequote_ctlesc_pairs / IFS_GLUE
//! strip), the walker tail restores DATA_* carriers to the characters
//! they denote, and decode_to_visible_text (locale.rs) decodes the rest
//! — so `V='<0x14>'` stores a backslash and `V='<0x11>'` stores
//! nothing. The `$'\xXX'` form is immune because decode_ansi_c_quoted +
//! push_ansi_c_byte (lexer/ansi.rs) re-encodes every decoded carrier
//! byte as a raw-byte marker pair (E000+E001+byte) at entry, per the
//! registry's user_reachable contract (markers.rs) — the single/double
//! quote and unquoted dequote paths do not. The unquoted path had a
//! second leak: remove_shell_quotes_inner_cursor's trigger-free fast
//! path returned the raw word verbatim, so a word with no quote/escape
//! trigger but a literal carrier byte (`V=<0x11>`) never reached the
//! tagging walk at all.
//!
//! GNU owner: parse.y:5694-5706 got_character/got_escaped_character — GNU
//! inserts CTLESC before a CTLESC/CTLNUL byte that arrives as data, so
//! its own markers can never be confused with user bytes; subst.c:4807
//! dequote_string removes only the CTLESC bytes; subst.c:11882 case
//! `'\''` hands a single-quoted span to add_quoted_string as literal
//! data. GNU ground truth (bash 5.3.9, script-file probes, all six
//! shapes x all 32 bytes): every byte round-trips as one byte, LEN=1.
//! Found by the #2902 closing chain (modernish test 002 per-CC
//! isolation, eco-2902-closing4 REPORT-CLOSING.md §6).

use std::io::Write;
use std::process::{Command, Stdio};

/// The 32 control bytes modernish defines as CC constants.
fn cc_bytes() -> Vec<u8> {
    let mut bytes: Vec<u8> = (0x01..=0x1f).collect();
    bytes.push(0x7f);
    bytes
}

fn run_driver(tag: &str, driver: &[u8]) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-3816-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let mut file = std::fs::File::create(dir.join("driver.sh")).expect("write driver");
    file.write_all(driver).expect("write driver");
    drop(file);
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("driver.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// One `od -An -tx1` line plus one LEN line per byte, exactly as GNU
/// prints them (`" 14\nLEN=1\n"`).
fn expected_probe_output(bytes: &[u8]) -> String {
    let mut out = String::new();
    for b in bytes {
        out.push_str(&format!(" {b:02x}\nLEN=1\n"));
    }
    out
}

fn probe_sq_assign(bytes: &[u8]) -> Vec<u8> {
    let mut driver = Vec::new();
    for b in bytes {
        let ch = char::from(*b);
        driver.extend_from_slice(format!("V='{ch}'\n").as_bytes());
        driver.extend_from_slice(b"printf '%s' \"$V\" | od -An -tx1\n");
        driver.extend_from_slice(b"echo \"LEN=${#V}\"\n");
    }
    driver
}

fn probe_dq_assign(bytes: &[u8]) -> Vec<u8> {
    let mut driver = Vec::new();
    for b in bytes {
        let ch = char::from(*b);
        driver.extend_from_slice(format!("V=\"{ch}\"\n").as_bytes());
        driver.extend_from_slice(b"printf '%s' \"$V\" | od -An -tx1\n");
        driver.extend_from_slice(b"echo \"LEN=${#V}\"\n");
    }
    driver
}

fn probe_unq_assign(bytes: &[u8]) -> Vec<u8> {
    // TAB (0x09) is IFS whitespace and LF (0x0a) ends the line: neither
    // can appear as a bare unquoted byte, so they are skipped here (they
    // are covered by the quoted shapes). CR (0x0d) is skipped too: a bare
    // `V=<CR>` line ends in the byte sequence CR LF, which the script
    // reader's line splitting consumes as a CRLF line ending before the
    // lexer ever sees the byte (on Windows the CR vanishes and the value
    // is empty; on Linux it survives) — a line-ending face separate from
    // #3816, still pinned for 0x0d by the quoted shapes.
    let mut driver = Vec::new();
    for b in bytes {
        if *b == 0x09 || *b == 0x0a || *b == 0x0d {
            continue;
        }
        let ch = char::from(*b);
        driver.extend_from_slice(format!("V={ch}\n").as_bytes());
        driver.extend_from_slice(b"printf '%s' \"$V\" | od -An -tx1\n");
        driver.extend_from_slice(b"echo \"LEN=${#V}\"\n");
    }
    driver
}

fn probe_unq_word(bytes: &[u8]) -> Vec<u8> {
    // Unquoted plain-word position: the byte as a bare argument word.
    // `printf '%s'` shows the argument the lexer delivered; `set --`
    // plus `${#1}` gives its length without any assignment path. Same
    // skips as probe_unq_assign (TAB/LF/CR cannot appear bare here).
    // This pins the remove_shell_quotes_inner_cursor fast path in
    // non-assignment position: a trigger-free word with a literal
    // carrier byte must not be returned verbatim.
    let mut driver = Vec::new();
    for b in bytes {
        if *b == 0x09 || *b == 0x0a || *b == 0x0d {
            continue;
        }
        let ch = char::from(*b);
        driver.extend_from_slice(format!("printf '%s' {ch} | od -An -tx1\n").as_bytes());
        driver.extend_from_slice(format!("set -- {ch}\n").as_bytes());
        driver.extend_from_slice(b"echo \"LEN=${#1}\"\n");
    }
    driver
}

fn probe_esc_assign(bytes: &[u8]) -> Vec<u8> {
    // Backslash-LF is a line continuation, so 0x0a is skipped here. CR
    // (0x0d) is skipped too: `V=\<CR>` ends in CR LF, which the script
    // reader consumes as a CRLF line ending, leaving a bare `V=\` that
    // continues onto the next line (same separate line-ending face as
    // probe_unq_assign; 0x0d is still pinned by the quoted shapes).
    let mut driver = Vec::new();
    for b in bytes {
        if *b == 0x0a || *b == 0x0d {
            continue;
        }
        let ch = char::from(*b);
        driver.extend_from_slice(format!("V=\\{ch}\n").as_bytes());
        driver.extend_from_slice(b"printf '%s' \"$V\" | od -An -tx1\n");
        driver.extend_from_slice(b"echo \"LEN=${#V}\"\n");
    }
    driver
}

fn probe_sq_word(bytes: &[u8]) -> Vec<u8> {
    let mut driver = Vec::new();
    for b in bytes {
        let ch = char::from(*b);
        driver.extend_from_slice(format!("printf '%s' '{ch}' | od -An -tx1\n").as_bytes());
    }
    driver
}

fn probe_ansi_assign(bytes: &[u8]) -> Vec<u8> {
    let mut driver = Vec::new();
    for b in bytes {
        driver.extend_from_slice(format!("V=$'\\x{b:02x}'\n").as_bytes());
        driver.extend_from_slice(b"printf '%s' \"$V\" | od -An -tx1\n");
        driver.extend_from_slice(b"echo \"LEN=${#V}\"\n");
    }
    driver
}

/// The reported shape: `CCxx='<byte>'` single-quoted assignments.
/// Base 969230b9 fails 10 of 32 (11 13 14 16 17 18 1A 1C 1D 1F).
#[test]
fn literal_single_quoted_assignment_keeps_all_32_control_bytes() {
    let bytes = cc_bytes();
    let (stdout, stderr, rc) = run_driver("sq-assign", &probe_sq_assign(&bytes));
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(stdout, expected_probe_output(&bytes));
}

/// Same 10 bytes break in double quotes too (same bare-byte entry).
#[test]
fn literal_double_quoted_assignment_keeps_all_32_control_bytes() {
    let bytes = cc_bytes();
    let (stdout, stderr, rc) = run_driver("dq-assign", &probe_dq_assign(&bytes));
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(stdout, expected_probe_output(&bytes));
}

/// Same 10 bytes break unquoted too (TAB/LF skipped: structural).
#[test]
fn literal_unquoted_assignment_keeps_control_bytes() {
    let bytes: Vec<u8> = cc_bytes()
        .into_iter()
        .filter(|b| *b != 0x09 && *b != 0x0a && *b != 0x0d)
        .collect();
    let (stdout, stderr, rc) = run_driver("unq-assign", &probe_unq_assign(&bytes));
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(stdout, expected_probe_output(&bytes));
}

/// Unquoted plain word: same fast-path leak, non-assignment position.
/// Base fails the same 10 bytes (the trigger-free word was returned
/// verbatim by remove_shell_quotes_inner_cursor's fast path).
#[test]
fn literal_unquoted_word_keeps_control_bytes() {
    let bytes: Vec<u8> = cc_bytes()
        .into_iter()
        .filter(|b| *b != 0x09 && *b != 0x0a && *b != 0x0d)
        .collect();
    let (stdout, stderr, rc) = run_driver("unq-word", &probe_unq_word(&bytes));
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(stdout, expected_probe_output(&bytes));
}

/// Same 10 bytes break backslash-escaped too (LF skipped: continuation).
#[test]
fn literal_backslash_escaped_assignment_keeps_control_bytes() {
    let bytes: Vec<u8> = cc_bytes()
        .into_iter()
        .filter(|b| *b != 0x0a && *b != 0x0d)
        .collect();
    let (stdout, stderr, rc) = run_driver("esc-assign", &probe_esc_assign(&bytes));
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(stdout, expected_probe_output(&bytes));
}

/// Plain-word position: 5 of the 32 break there (11 17 18 1A 1F) — the
/// assignment boundary claims the other five, argv claims these.
#[test]
fn literal_single_quoted_word_keeps_all_32_control_bytes() {
    let bytes = cc_bytes();
    let mut expected = String::new();
    for b in &bytes {
        expected.push_str(&format!(" {b:02x}\n"));
    }
    let (stdout, stderr, rc) = run_driver("sq-word", &probe_sq_word(&bytes));
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(stdout, expected);
}

/// Control: `$'\xXX'` is already correct on base (32/32) and must stay so.
#[test]
fn ansi_c_assignment_keeps_all_32_control_bytes() {
    let bytes = cc_bytes();
    let (stdout, stderr, rc) = run_driver("ansi-assign", &probe_ansi_assign(&bytes));
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(stdout, expected_probe_output(&bytes));
}

/// Guard: the existing single-quote data carriers must keep working —
/// a literal `$`, backtick, `"` and `\` inside single quotes are data,
/// not syntax. Tagging literal carrier bytes must not disturb the
/// deliberate `$`->DATA_DOLLAR / `` ` ``->DATA_BACKTICK /
/// `"`->DATA_DQUOTE / `\`->DATA_BACKSLASH mappings.
#[test]
fn single_quoted_metacharacters_stay_literal_data() {
    let driver = b"V='a$b`c\"d\\e'\nprintf '%s' \"$V\" | od -An -tx1\necho \"LEN=${#V}\"\n";
    let (stdout, stderr, rc) = run_driver("sq-meta", driver);
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(stderr, "");
    // a $ b ` c " d \ e  ==  61 24 62 60 63 22 64 5c 65, LEN=9.
    assert_eq!(stdout, " 61 24 62 60 63 22 64 5c 65\nLEN=9\n");
}

/// End-to-end modernish shape: all 32 CC constants concatenated must
/// be 32 bytes (base prints 29: the 10 damaged bytes collapse/expand).
#[test]
fn controlchars_concatenation_is_32_bytes() {
    let bytes = cc_bytes();
    let mut driver = Vec::new();
    for b in &bytes {
        let ch = char::from(*b);
        driver.extend_from_slice(format!("V{b:02x}='{ch}'\n").as_bytes());
    }
    driver.extend_from_slice(b"CC=\"\"; ");
    for b in &bytes {
        driver.extend_from_slice(format!("CC=\"$CC$V{b:02x}\"; ").as_bytes());
    }
    driver.extend_from_slice(b"\nprintf '%s' \"$CC\" | od -An -tx1\necho \"LEN=${#CC}\"\n");
    let mut expected = String::new();
    for chunk in bytes.chunks(16) {
        expected.push(' ');
        expected.push_str(
            &chunk
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join(" "),
        );
        expected.push('\n');
    }
    expected.push_str("LEN=32\n");
    let (stdout, stderr, rc) = run_driver("cc-concat", &driver);
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(stdout, expected);
}
