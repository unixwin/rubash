//! rubash#3103 — unquoted splitting in command-word position (for-list /
//! `set --` / command arguments) loses data carrier bytes: a data \x1c is
//! eaten as the IFS_GLUE protocol marker and a data \x1f is folded to '$'.
//!
//! The array-assignment path was fixed by rubash#3004: the byte-domain
//! splitter decodes raw-byte marker pairs WITH provenance
//! (`decode_raw_byte_markers_with_glue`), so a pair payload is data and
//! only a bare transport '\u{1c}' char is protocol. The command-word
//! expansion walker instead decoded the pairs into bare chars before
//! splitting, so the char-domain splitter could only read the decoded
//! \x1c as glue and the decoded \x1f as DATA_DOLLAR. GNU: subst.c:12377
//! word_split / list_string splits the expansion result on IFS chars
//! only; a data \x1c-\x1f byte in the result is an ordinary character
//! end to end.
//!
//! Harness: `rubash -c` via `CARGO_BIN_EXE_rubash`, byte-level stdout
//! assertions, `LC_ALL=C.UTF-8` pinned on the child (hermetic under any
//! ambient locale), mirroring tests/issue3004's harness.

use std::process::{Command, Stdio};

fn run(script: &str) -> Vec<u8> {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash")
        .wait_with_output()
        .expect("wait rubash");
    assert!(
        output.status.success(),
        "rubash -c {script:?} exited with {}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn expect(script: &str, expected: &[u8], ctx: &str) {
    let out = run(script);
    assert_eq!(
        out,
        expected,
        "{ctx}: got {:?}, want {:?}",
        String::from_utf8_lossy(&out),
        String::from_utf8_lossy(expected)
    );
}

/// One cell per carrier byte \x1c-\x1f and per command-word split shape:
/// a value `x<B>y\nz<B>w`, split on IFS newline, must yield exactly the
/// two fields with the byte intact in both.
fn check_shape(shape: &str, octal: &str, byte: u8) {
    let script = match shape {
        "for" => format!(
            "raw=$(printf 'x\\{octal}y\\nz\\{octal}w'); \
             k() {{ local IFS=$'\\n'; for w in $1; do printf '<%s>' \"$w\"; done; }}; \
             k \"$raw\""
        ),
        "set" => format!(
            "raw=$(printf 'x\\{octal}y\\nz\\{octal}w'); \
             m() {{ local IFS=$'\\n'; set -- $1; printf '<%s><%s>' \"$1\" \"$2\"; }}; \
             m \"$raw\""
        ),
        "args" => format!(
            "raw=$(printf 'x\\{octal}y\\nz\\{octal}w'); \
             show() {{ printf '<%s><%s>' \"$1\" \"$2\"; }}; \
             k() {{ local IFS=$'\\n'; show $1; }}; \
             k \"$raw\""
        ),
        _ => unreachable!(),
    };
    let mut expected = Vec::new();
    expected.extend_from_slice(b"<x");
    expected.push(byte);
    expected.extend_from_slice(b"y><z");
    expected.push(byte);
    expected.extend_from_slice(b"w>");
    expect(
        &script,
        &expected,
        &format!("{shape} split must keep data byte 0x{byte:02x} in both fields"),
    );
}

#[test]
fn for_list_split_keeps_carrier_bytes() {
    for (octal, byte) in [("034", 0x1cu8), ("035", 0x1d), ("036", 0x1e), ("037", 0x1f)] {
        check_shape("for", octal, byte);
    }
}

#[test]
fn set_positional_split_keeps_carrier_bytes() {
    for (octal, byte) in [("034", 0x1cu8), ("035", 0x1d), ("036", 0x1e), ("037", 0x1f)] {
        check_shape("set", octal, byte);
    }
}

#[test]
fn command_args_split_keeps_carrier_bytes() {
    for (octal, byte) in [("034", 0x1cu8), ("035", 0x1d), ("036", 0x1e), ("037", 0x1f)] {
        check_shape("args", octal, byte);
    }
}

#[test]
fn combined_value_matches_gnu_probe() {
    // The original #3103 evidence shape verbatim (probe-ml2): one value
    // carrying \x1c in the first field and \x1f in the second.
    expect(
        "raw=$(printf 'x\\034y\\nz\\037w'); \
         k() { local IFS=$'\\n'; for w in $1; do printf '<%s>' \"$w\"; done; }; \
         k \"$raw\"",
        b"<x\x1cy><z\x1fw>",
        "combined probe value must survive the for-list split byte-for-byte",
    );
}

#[test]
fn split_guard_quoted_value_not_split() {
    // Guard: a QUOTED expansion in the same positions must not split at
    // all, and its carrier bytes are data regardless of the split path.
    expect(
        "raw=$(printf 'x\\034y\\nz\\037w'); \
         k() { local IFS=$'\\n'; for w in \"$1\"; do printf '<%s>' \"$w\"; done; }; \
         k \"$raw\"",
        b"<x\x1cy\nz\x1fw>",
        "quoted for-list value stays one field with bytes intact",
    );
}

#[test]
fn split_guard_plain_value_still_splits() {
    // Guard: ordinary values without carrier bytes split exactly as
    // before on all three shapes.
    expect(
        "k() { local IFS=$'\\n'; for w in $1; do printf '<%s>' \"$w\"; done; }; \
         k 'aa
bb'",
        b"<aa><bb>",
        "plain for-list split is unchanged",
    );
}
