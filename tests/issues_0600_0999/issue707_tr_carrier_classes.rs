// niubash#707: the inline `tr` fast path ran set membership over the shell's
// TRANSPORT text. Carrier bytes (0x0C, 0x1B, 0x14..0x1F, ...) travel between
// the lexer and the executor as U+E000+U+E0xx raw-byte marker pairs, so a set
// built from the real characters (`[:cntrl:]`, `[:space:]`) never matched
// them and the carried control bytes escaped translation.
//
// Same payload-decoding class as niubash#383 (the inline `wc`): decode once
// with `shell_text_to_raw_bytes` before the class lookup. The external
// WinuxCmd `tr` and GNU coreutils agree on every payload below.

use std::process::Command;

fn run(cmd: &str) -> (Vec<u8>, i32) {
    let exe = env!("CARGO_BIN_EXE_rubash");
    let out = Command::new(exe)
        .arg("-c")
        .arg(cmd)
        .output()
        .expect("spawn rubash");
    (out.stdout, out.status.code().unwrap_or(-1))
}

/// ESC (0x1B) is carrier-encoded; `[:cntrl:]` must still map it.
#[test]
fn cntrl_class_maps_carried_esc() {
    let (out, rc) = run("printf 'a\\033b\\nc\\n' | tr '[:cntrl:]' X");
    assert_eq!((out.as_slice(), rc), (b"aXbXcX".as_slice(), 0));
}

/// Form feed (0x0C) is carrier-encoded AND whitespace; `[:space:]` maps it.
#[test]
fn space_class_maps_carried_form_feed() {
    let (out, rc) = run("printf 'a\\014b\\n' | tr '[:space:]' _");
    assert_eq!((out.as_slice(), rc), (b"a_b_".as_slice(), 0));
}

/// 0x0B (VT) is not carried and already mapped; 0x0C (FF) is carried and
/// used to escape — both are `[:cntrl:]`, so only the carrier set bytes leak.
#[test]
fn only_carrier_set_bytes_escaped() {
    let (out, rc) = run("printf 'a\\013\\014b\\n' | tr '[:cntrl:]' X");
    assert_eq!((out.as_slice(), rc), (b"aXXbX".as_slice(), 0));
}

/// The ordinary ranged translate must be untouched by the decode step.
#[test]
fn plain_translate_no_regression() {
    let (out, rc) = run("printf 'Hello\\n' | tr a-z A-Z");
    assert_eq!((out.as_slice(), rc), (b"HELLO\n".as_slice(), 0));
}

/// The `$( ... | tr ... )` arm (command_substitution_values.rs) shares the
/// same payload decode.
#[test]
fn command_substitution_decodes_carried_bytes() {
    let (out, rc) = run("x=$(printf 'a\\033b\\n' | tr '[:cntrl:]' X); printf '%s' \"$x\"");
    assert_eq!((out.as_slice(), rc), (b"aXbX".as_slice(), 0));
}
