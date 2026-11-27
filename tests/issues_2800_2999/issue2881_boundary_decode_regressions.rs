#![cfg(unix)]
//! #2881 (PUA/carrier sweep F1, fix item E1): the decoder used at the
//! external boundary, `decode_raw_byte_markers_to_byte_chars`, had no
//! `LITERAL_CHAR_ESCAPE` (U+E400) arm, so a value containing a typed
//! registry-zone char reached real child processes as the U+E400 + char
//! escape pair instead of the raw char. GNU bash 5.3.20 passes the raw
//! char (sweep battery cells c08/c09, byte-verified).
//!
//! These pins spawn the binary and read the bytes a real child
//! (/usr/bin/env, /usr/bin/printf) actually receives, at two of the
//! decoder's three call sites (child argv, child env / `env` builtin
//! env). The third call site (`child_env_value`) shares the same
//! decoder and is covered by the unit parity test in
//! `src/executor/substitution_metadata.rs`.

use std::io::Write;
use std::process::{Command, Stdio};

struct Run {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    code: Option<i32>,
}

fn run(script: &str) -> Run {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(script.as_bytes())
        .expect("write script");
    let out = child.wait_with_output().expect("wait rubash");
    Run {
        stdout: out.stdout,
        stderr: out.stderr,
        code: out.status.code(),
    }
}

fn assert_clean(run: &Run, ctx: &str) {
    assert_eq!(run.code, Some(0), "{ctx}: exit code");
    assert!(
        run.stderr.is_empty(),
        "{ctx}: stderr must be empty: {:?}",
        String::from_utf8_lossy(&run.stderr)
    );
}

fn contains_seq(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// U+E314 in UTF-8 — the raw char GNU hands to the child.
const RAW_E314: &[u8] = "\u{e314}".as_bytes();
/// U+E400 U+E314 in UTF-8 — the escape pair that must never leak.
const PAIR_E314: &[u8] = "\u{e400}\u{e314}".as_bytes();

/// Battery c08 shape, child env boundary (`child_env_value` caller set /
/// spawn env): an exported typed registry char arrives raw.
#[test]
fn exported_typed_registry_char_reaches_child_env_raw() {
    let run = run("v=$'\\x1cZ\\x1b'\nexport v\nw=$'\\uE314'\nexport w\n/usr/bin/env\n");
    assert_clean(&run, "c08 child env");
    // Control: raw C0 bytes already crossed correctly before the fix.
    let mut control = b"v=".to_vec();
    control.extend_from_slice(&[0x1c, b'Z', 0x1b, b'\n']);
    assert!(
        contains_seq(&run.stdout, &control),
        "control: child env must carry raw C0 bytes: {:?}",
        String::from_utf8_lossy(&run.stdout)
    );
    let mut want = b"w=".to_vec();
    want.extend_from_slice(RAW_E314);
    want.push(b'\n');
    assert!(
        contains_seq(&run.stdout, &want),
        "child env must carry the raw U+E314 after w=: {:?}",
        String::from_utf8_lossy(&run.stdout)
    );
    assert!(
        !contains_seq(&run.stdout, PAIR_E314),
        "the U+E400 escape pair must not leak into child env: {:?}",
        String::from_utf8_lossy(&run.stdout)
    );
}

/// Battery c09 shape, child argv boundary (`path.rs` caller): an argv
/// word carrying a typed registry char arrives raw.
#[test]
fn typed_registry_char_reaches_child_argv_raw() {
    let argv = run("w=$'\\uE314'\n/usr/bin/printf '%s' \"$w\"\n");
    assert_clean(&argv, "c09 child argv");
    assert_eq!(
        argv.stdout,
        RAW_E314.to_vec(),
        "child argv must be the raw U+E314 bytes, not the escape pair"
    );

    // Controls: raw-byte values already crossed correctly before the fix.
    let ctl1 = run("v=$'\\x1cZ'\n/usr/bin/printf '%s' \"$v\"\n");
    assert_clean(&ctl1, "c09 control 1");
    assert_eq!(ctl1.stdout, vec![0x1c, b'Z']);
    let ctl2 = run("z=$'\\x01\\x02'\n/usr/bin/printf '%s' \"$z\"\n");
    assert_clean(&ctl2, "c09 control 2");
    assert_eq!(ctl2.stdout, vec![0x01, 0x02]);
}

/// The `env` builtin boundary (`apply_env_command_environment` caller):
/// an assignment-prefix value carrying a typed registry char reaches
/// the env-builtin's child raw.
#[test]
fn env_builtin_value_with_typed_registry_char_reaches_child_raw() {
    let run = run("env W=$'\\uE314' /usr/bin/env\n");
    assert_clean(&run, "env builtin child env");
    let mut want = b"W=".to_vec();
    want.extend_from_slice(RAW_E314);
    want.push(b'\n');
    assert!(
        contains_seq(&run.stdout, &want),
        "env builtin must hand the raw U+E314 to its child: {:?}",
        String::from_utf8_lossy(&run.stdout)
    );
    assert!(
        !contains_seq(&run.stdout, PAIR_E314),
        "the U+E400 escape pair must not leak via the env builtin: {:?}",
        String::from_utf8_lossy(&run.stdout)
    );
}
