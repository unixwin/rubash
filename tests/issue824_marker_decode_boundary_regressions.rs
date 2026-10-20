//! #824: marker debug_assert family — user-literal text panicked debug
//! builds at two more decode boundaries after #802 (HD1 in `declare -p`
//! value rendering; PARAM_WORD_BACKSLASH_GUARD in xtrace).
//!
//! Root cause (both sites, one class): the output-side golden asserts
//! inside `decode_to_visible_text` read "registered codepoint in output ⇒
//! decode failed". The #729 carrier-consumer audit showed every registered
//! codepoint has a legitimate user-data path to that output: C0 bytes ride
//! raw-byte marker pairs (registry `user_reachable` contract), registry-zone
//! PUA chars ride the E400 literal-char escape (rubash#735), and the named
//! ASCII markers (`__RUBASH_HD1__`, `__RUBASH_CSB1_`) are simply typeable.
//! The asserts could only ever fire on well-formed user data.
//!
//! These tests spawn the binary (the dev-profile `cargo test` build arms
//! debug_asserts inside it, so the pre-fix code panics here) and pin the
//! GNU-oracle bytes; release builds must produce the identical bytes (zero
//! non-debug change). GNU oracle = WSL GNU bash 5.3.0 stdin method — each
//! expected byte block was captured from it.

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
        "{ctx}: stderr must be empty (a debug_assert panic lands here): {:?}",
        String::from_utf8_lossy(&run.stderr)
    );
}

/// Same as [`assert_clean`] for xtrace runs: the trace itself lives on
/// stderr, so panic-freedom is asserted by exit code plus the exact trace
/// bytes (a panic would truncate them), not by emptiness.
fn assert_exit_ok(run: &Run, ctx: &str) {
    assert_eq!(run.code, Some(0), "{ctx}: exit code");
}

/// Site 1: `x=a__RUBASH_HD1__b; declare -p x` panicked a debug build with
/// "QUOTED_HEREDOC_MARKER leaked to output". The marker string is the
/// user's own data here; GNU 5.3.0 prints it verbatim.
#[test]
fn hd1_user_literal_survives_declare_p_value_rendering() {
    let run = run("x=a__RUBASH_HD1__b; declare -p x\n");
    assert_clean(&run, "HD1 declare -p");
    assert_eq!(
        run.stdout,
        b"declare -- x=\"a__RUBASH_HD1__b\"\n".to_vec(),
        "GNU oracle: the typed marker text survives the value-rendering decode"
    );
}

/// Same boundary, the comsub payload prefix: `x='__RUBASH_CSB1_41;'
/// ; declare -p x` panicked with "COMSUB_PAYLOAD_PREFIX leaked to output".
/// GNU 5.3.0 prints the text verbatim.
#[test]
fn comsub_payload_prefix_user_literal_survives_declare_p() {
    let run = run("x='__RUBASH_CSB1_41;'; declare -p x\n");
    assert_clean(&run, "CSB1 declare -p");
    assert_eq!(
        run.stdout,
        b"declare -- x=\"__RUBASH_CSB1_41;\"\n".to_vec(),
        "GNU oracle: the typed payload-prefix text survives verbatim"
    );
}

/// A non-marker lookalike must stay byte-identical (GNU 5.3.0 captured).
#[test]
fn hd1_lookalike_is_untouched() {
    let run = run("x=a__RUBASH_HD1_nearly__b; declare -p x\n");
    assert_clean(&run, "HD1 lookalike");
    assert_eq!(
        run.stdout,
        b"declare -- x=\"a__RUBASH_HD1_nearly__b\"\n".to_vec()
    );
}

/// Site 2: xtrace of guard-equal user data panicked a debug build with
/// "PARAM_WORD_BACKSLASH_GUARD leaked to output" — the U+E314 char is the
/// user's own `$'\uE314Z'` data, restored verbatim through the E400
/// literal-char escape (rubash#735). GNU 5.3.0 traces it raw twice (the
/// assignment and the echo word).
#[test]
fn xtrace_traces_guard_equal_user_data_verbatim() {
    let script = "set -x\nv=$'\\uE314Z'\necho \"$v\"\nset +x\n";
    let run = run(script);
    assert_exit_ok(&run, "xtrace guard data");
    assert_eq!(
        run.stdout,
        "\u{E314}Z\n".as_bytes().to_vec(),
        "stdout: the PUA char round-trips literally"
    );
    assert_eq!(
        run.stderr,
        "+ v=\u{E314}Z\n+ echo \u{E314}Z\n+ set +x\n"
            .as_bytes()
            .to_vec(),
        "GNU oracle: xtrace carries the user char raw, no panic, no escape"
    );
}

/// Same audit sweep, C0 family: a raw 0x11 byte is the user's own data
/// (raw-byte marker pair at entry); GNU 5.3.0 traces it as $'\021'.
#[test]
fn xtrace_traces_c0_user_byte_as_ansic_quote() {
    let script = "x=$'\\x11'\nset -x\necho \"$x\"\nset +x\n";
    let run = run(script);
    assert_exit_ok(&run, "xtrace C0 data");
    assert_eq!(run.stdout, b"\x11\n".to_vec());
    assert_eq!(
        run.stderr,
        b"+ echo $'\\021'\n+ set +x\n".to_vec(),
        "GNU oracle: the byte renders through the $'...' quoting arm"
    );
}

/// Assignment DATA_* sentinels (U+E301/E30A) and the #735 guard-escape
/// char (U+E401) are registry-zone PUA chars: user data must round-trip
/// verbatim through `declare -p` (GNU 5.3.0 prints each char raw inside
/// double quotes). The array-element arm shares the boundary
/// (`quote_array_element_value`), pinned by the scalar cases here.
#[test]
fn registry_zone_pua_user_data_roundtrips_declare_p() {
    let script =
        "x=$'\\uE301Y'; declare -p x\nx=$'\\uE30AY'; declare -p x\nx=$'\\uE401W'; declare -p x\n";
    let run = run(script);
    assert_clean(&run, "PUA declare -p");
    let expected = format!(
        "declare -- x=\"\u{E301}Y\"\ndeclare -- x=\"\u{E30A}Y\"\ndeclare -- x=\"\u{E401}W\"\n"
    );
    assert_eq!(run.stdout, expected.as_bytes().to_vec(), "GNU oracle bytes");
}

/// The echo path must keep printing the literal marker text (GNU 5.3.0
/// captured). The printf `%s` arm's typed-`__RUBASH_CSB1_<hex>;` decode
/// divergence noted here at #824 time (`aA` instead of GNU's literal) was
/// fixed by #840 — the payload introducer moved off typeable ASCII to the
/// registry-zone U+E319 sentinel — and is pinned below.
#[test]
fn echo_prints_marker_text_verbatim() {
    let run = run("echo '__RUBASH_HD1__'\n");
    assert_clean(&run, "echo HD1");
    assert_eq!(run.stdout, b"__RUBASH_HD1__\n".to_vec());
}

/// rubash#840 divergence 2 end-to-end: a typed `__RUBASH_CSB1_<hex>;` is
/// pure user data at printf/echo — GNU 5.3.0 prints it verbatim.
#[test]
fn typed_csb1_lookalike_prints_verbatim() {
    let run = run("printf '%s\n' '__RUBASH_CSB1_41;'\necho '__RUBASH_CSB1_42;'\n");
    assert_clean(&run, "CSB1 printf/echo");
    assert_eq!(
        run.stdout,
        b"__RUBASH_CSB1_41;\n__RUBASH_CSB1_42;\n".to_vec(),
        "GNU oracle: the typed lookalike is literal text"
    );
}

/// The #802 reserved-string stance is unchanged by the #824 sweep: the
/// compound-assignment carrier is consumed on the visible-text boundary
/// (`a__RUBASH_CA1__b` renders as `ab`), which diverges from GNU's literal
/// `a__RUBASH_CA1__b` by design — user-typed CA1 text is indistinguishable
/// from the parser's carrier there, the same reserved-string trade GNU
/// makes for its own CTLESC byte. Pinned so the stance stays a decision.
#[test]
fn ca1_reserved_string_stance_unchanged() {
    let run = run("x=a__RUBASH_CA1__b; declare -p x\n");
    assert_clean(&run, "CA1 declare -p");
    assert_eq!(
        run.stdout,
        b"declare -- x=\"ab\"\n".to_vec(),
        "the #802 consume is untouched (GNU would print the literal text)"
    );
}
