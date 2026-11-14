//! rubash#3433 — the `bad substitution` diagnostic echoes the offending
//! word with a raw internal PUA marker (U+E31B, UTF-8 `EE 8C 9B`) left in
//! it, whenever the word is assignment-like (`<name>=` immediately before
//! the expansion) AND the bad expansion starts with `-` (e.g. `${-x}`,
//! `${-n-unset}`). GNU prints the word verbatim (subst.c:10278
//! `report_error(_("%s: bad substitution"), string)` cites the word being
//! expanded).
//!
//! Root cause: the pre-expansion marking pass
//! (`mark_quoted_ifs_from_raw`, command_prepare.rs) inserts
//! `QUOTED_EXPANSION_SITE` (U+E31B) before a `$`/backtick expansion whose
//! site lay inside a quoted segment, so the walkers can IFS-protect that
//! expansion's result span (rubash#2706; GNU subst.c:11862
//! `add_quoted_string`). The walkers consume the marker on sight, but the
//! bad-substitution diagnostic cites the word recorded BEFORE the walk
//! (`PendingDiagnosticWordGuard`, embedded_mutations.rs) — the marked word
//! — and renders it through `bad_substitution_display`
//! (`decode_to_visible_text`), which drops the sibling marker U+E31A
//! (COMPOUND_ASSIGNMENT_MARKER) but has no arm for U+E31B, so the marker
//! falls through to stderr verbatim.
//!
//! Trigger boundary (GNU oracle 5.3.20, probed): the leak needs a valid
//! identifier immediately followed by `=` immediately before the expansion
//! (`a=`, `aa=`, `A=` leak; `a${-x}`, `=${-x}`, `a-b${-x}` do not). A
//! genuine assignment statement (`x="a=${-x}"`) is clean on both shells.
//!
//! Expectations verified against GNU bash 5.3.20 (byte tables in the lane
//! REPORT; seed `seed_r108c.sh` in the r108 hunt run: GNU 14/14, verdict
//! rubash 4 FAIL + 10 PASS).
//!
//! Harness: `rubash -c` via `CARGO_BIN_EXE_rubash`, stderr byte-level
//! assertions. `LC_ALL=C.UTF-8` is set on the child explicitly so the pin
//! is hermetic under any ambient locale. The diagnostic prefix carries the
//! test binary's argv[0], so assertions match the message body, not the
//! full line. Bad substitution aborts the `-c` script on both shells
//! (rc 1, empty stdout) — asserted for the leak shapes.

use std::process::{Command, Stdio};

/// UTF-8 bytes of the leaked marker U+E31B.
const PUA_MARKER: &[u8] = "\u{E31B}".as_bytes();

fn run(script: &str) -> (Vec<u8>, Vec<u8>, i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    (
        output.stdout,
        output.stderr,
        output.status.code().unwrap_or(-1),
    )
}

fn check_no_leak(script: &str, message: &str) {
    let (stdout, stderr, rc) = run(script);
    let stderr_text = String::from_utf8_lossy(&stderr);
    assert_eq!(rc, 1, "{script:?}: rc {rc}, stderr: {stderr_text}");
    assert!(
        stdout.is_empty(),
        "{script:?}: stdout must be empty, got {stdout:?}"
    );
    assert!(
        stderr_text.contains(message),
        "{script:?}: stderr lacks [{message}], got [{stderr_text}]"
    );
    assert!(
        !stderr.windows(PUA_MARKER.len()).any(|w| w == PUA_MARKER),
        "{script:?}: stderr leaks PUA marker U+E31B: [{stderr_text}]"
    );
}

fn check_control(script: &str, message: &str) {
    let (_stdout, stderr, _rc) = run(script);
    let stderr_text = String::from_utf8_lossy(&stderr);
    assert!(
        stderr_text.contains(message),
        "{script:?}: stderr lacks [{message}], got [{stderr_text}]"
    );
    assert!(
        !stderr.windows(PUA_MARKER.len()).any(|w| w == PUA_MARKER),
        "{script:?}: stderr leaks PUA marker U+E31B: [{stderr_text}]"
    );
}

#[test]
fn bad_substitution_assignment_like_word_has_no_pua_marker() {
    // Seed shapes (r108 R108-C): the four leak shapes.
    check_no_leak("echo \"a=${-x}\"", "a=${-x}: bad substitution");
    check_no_leak("echo \"a=b${-x}c\"", "a=b${-x}c: bad substitution");
    check_no_leak("echo \"k=v${-x}\"", "k=v${-x}: bad substitution");
    check_no_leak("echo \"n=${-n-unset}\"", "n=${-n-unset}: bad substitution");
    // Trigger-boundary morphs: other `<name>=` prefixes leak the same way.
    check_no_leak("echo \"aa=${-x}\"", "aa=${-x}: bad substitution");
    check_no_leak("echo \"A=${-x}\"", "A=${-x}: bad substitution");
    check_no_leak("echo \"a=${-x}b\"", "a=${-x}b: bad substitution");
    // Two quoted sites in one word leak two markers on the base.
    check_no_leak("echo \"a=${-x} ${-y}\"", "a=${-x} ${-y}: bad substitution");
}

#[test]
fn bad_substitution_controls_stay_clean() {
    // No `=` before the expansion.
    check_control("echo \"ab${-x}\"", "ab${-x}: bad substitution");
    check_control("echo \"${-x}\"", "${-x}: bad substitution");
    check_control("echo \"a${-x}\"", "a${-x}: bad substitution");
    check_control("echo \"=${-x}\"", "=${-x}: bad substitution");
    check_control("echo \"a-b${-x}\"", "a-b${-x}: bad substitution");
    // `=` with a non-dash expansion.
    check_control("echo \"n=${1x}\"", "n=${1x}: bad substitution");
    // A genuine assignment statement is clean on both shells.
    check_control("x=\"a=${-x}\"", "a=${-x}: bad substitution");
}

#[test]
fn user_typed_pua_marker_still_round_trips() {
    // A user-typed U+E31B arrives E400-escaped at entry (push_literal_char)
    // and is user data, not protocol: the decoder must restore it, never
    // drop it (marker_leak_golden.rs pins the echo path; this pins that the
    // same decoder still emits it after the #3433 drop arm is added).
    let (stdout, _stderr, _rc) = run("echo $'\\uE31B'");
    assert_eq!(stdout, "\u{E31B}\n".as_bytes());
}
