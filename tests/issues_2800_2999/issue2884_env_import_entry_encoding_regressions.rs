//! rubash#2884 (PUA sweep F6): process-environment import was the one
//! entry point that stored incoming values WITHOUT entry encoding, so
//! inherited user values were read as live carrier protocol downstream.
//! GNU has no transport domain: `variables.c`
//! `initialize_shell_variables` stores environ verbatim and every view
//! shows the bytes as-is. The fix entry-encodes user values at the
//! import funnel (`Executor::new_inner`), at parity with the other
//! value-entry channels (`push_escaped_text_with_carriers`: stdin
//! lines, `read` input, comsub assignment RHS); `__RUBASH_*` internal
//! transport names keep their raw interpretation, and `BASH_FUNC_*%%`
//! entries ride the source channel (marker-zone escaping only, the
//! `bytes_to_script_text` parity).
//!
//! Expectations captured byte-for-byte from GNU bash 5.3.20 (the
//! oracle probe lives in the lane REPORT).

use std::process::Command;

fn run_rubash(script: &str, extra_env: &[(&str, String)]) -> String {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg("-c").arg(script);
    cmd.env_remove("BASH_ENV").env_remove("WINUXSH_ROOT");
    for (name, value) in extra_env {
        cmd.env(name, value);
    }
    let output = cmd.output().expect("run rubash");
    assert!(
        output.status.success(),
        "rubash failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

const PROBE: &str =
    "v=$IMPORTED; printf '%s' \"$v\" | od -An -tx1; echo \"len=${#v}\"; declare -p v";

/// Golden cell 1: a bare PARAM_WORD_BACKSLASH_GUARD (U+E314) inherited
/// from the parent is user data. Before the fix, expansion kept it but
/// `declare -p` printed `v=""` — the display decoder's guard arm ate
/// the bare registry char.
#[test]
fn imported_bare_guard_char_is_data() {
    let stdout = run_rubash(PROBE, &[("IMPORTED", "\u{E314}".to_string())]);
    assert_eq!(stdout, " ee 8c 94\nlen=1\ndeclare -- v=\"\u{E314}\"\n");
}

/// Golden cell 2: pair-shaped data (U+E400 + U+E314) inherited from
/// the parent is two data chars, not an escape pair. Before the fix it
/// collapsed to one char on expansion (`len=1`).
#[test]
fn imported_escape_pair_shape_is_two_data_chars() {
    let stdout = run_rubash(PROBE, &[("IMPORTED", "\u{E400}\u{E314}".to_string())]);
    assert_eq!(
        stdout,
        " ee 90 80 ee 8c 94\nlen=2\ndeclare -- v=\"\u{E400}\u{E314}\"\n"
    );
}

/// Golden cell 3: a raw ESC byte inherited from the parent prints
/// through the `$'...'` quoting form, exactly like a script-entered
/// value. Before the fix `declare -p` printed the raw ESC byte inside
/// plain quotes (`v="A<ESC>B"`).
#[test]
fn imported_raw_escape_byte_quotes_like_gnu() {
    let stdout = run_rubash(
        "v=$IMPORTED; declare -p v; printf '%s' \"$v\" | od -An -tx1",
        &[("IMPORTED", "A\u{1B}B".to_string())],
    );
    assert_eq!(stdout, "declare -- v=$'A\\EB'\n 41 1b 42\n");
}

/// Control (already passing before the fix): a lone payload-range char
/// (U+E014, the second half of a raw-byte marker pair with no
/// introducer before it) is kept verbatim.
#[test]
fn imported_lone_payload_char_is_kept() {
    let stdout = run_rubash(PROBE, &[("IMPORTED", "\u{E014}".to_string())]);
    assert_eq!(stdout, " ee 80 94\nlen=1\ndeclare -- v=\"\u{E014}\"\n");
}

/// Control: ordinary control bytes that are not carrier bytes (tab,
/// newline) ride verbatim, as they do on every other entry channel.
#[test]
fn imported_tab_and_newline_ride_verbatim() {
    let stdout = run_rubash(PROBE, &[("IMPORTED", "a\tb\nc".to_string())]);
    assert_eq!(
        stdout,
        " 61 09 62 0a 63\nlen=5\ndeclare -- v=$'a\\tb\\nc'\n"
    );
}

/// The function-source channel: a `BASH_FUNC_*%%` body whose string
/// literal holds a marker-zone char imports as a working function and
/// prints the char verbatim (GNU parent bodies are fresh source; the
/// source channel's entry escaping applies, not the data channel's).
#[test]
fn imported_function_body_marker_literal_survives() {
    let stdout = run_rubash(
        "greet",
        &[("BASH_FUNC_greet%%", "() { echo 'x\u{E314}y'; }".to_string())],
    );
    assert_eq!(stdout, "x\u{E314}y\n");
}

/// Round trip: an imported user value re-exports to a nested rubash
/// child as the raw bytes (the exit boundary decodes), and the child
/// imports it as data again — the pair-shaped value survives both
/// crossings with its length intact.
#[test]
fn imported_value_round_trips_through_nested_shell() {
    let bin = env!("CARGO_BIN_EXE_rubash");
    let script = format!(
        "\"{bin}\" -c 'v=$IMPORTED; echo \"inner len=${{#v}}\"'",
        bin = bin
    );
    let stdout = run_rubash(&script, &[("IMPORTED", "\u{E400}\u{E314}".to_string())]);
    assert_eq!(stdout, "inner len=2\n");
}
