//! Issue rubash#1011: an embedded `"${a[@]:off[:len]}"` inside a quoted
//! word did not word-split like GNU (joined into one word).
//!
//! GNU semantics (vendored tree `9c465866`, Bash-5.3 patch 20):
//! subst.c:10119-10128 parameter_brace_expand calls chk_atstar
//! (subst.c:7635-7639) on every valid array reference BEFORE the operator
//! switch, so a quoted `[@]`/`@` marks quoted_dollar_at even with the `:`
//! substring operator present; subst.c:9050 parameter_brace_substring then
//! keeps the @-list word boundaries (VT_ARRAYVAR/VT_POSPARMS arms,
//! subst.c:9101-9136: array_subrange/string_list_pos_params). The slice
//! therefore splits the enclosing word exactly like the unsliced list,
//! with affixes on the first/last element word:
//!   `f "A|${a[@]:1:2}|B"` is argc=2 (`[A|y]` `[z|B]`), not argc=1.
//! Standalone `"${a[@]:1:2}"` already fanned out via `array_at_word_values`;
//! only the embedded (mixed-word) shape diverged.
//!
//! Root cause: the raw-word segment scanner
//! (`quoted_positional_at_segments` in
//! src/executor/command_substitution_values.rs) only recognized the exact
//! `"${name[@]}"` / `${name[@]}` / `${@}` spans as word-list sources, so an
//! `@`-slice span fell through to the joining String path. The fix admits
//! `name[@]:off[:len]` and `@:off[:len]` spans (quoted-body and unquoted
//! top-level arms) and resolves them with the same slicing helpers as the
//! standalone paths (`array_subscript_range_values`,
//! `positional_parameter_substring_with_zero`).
//!
//! Expected outputs below are byte-verified against GNU Bash 5.3.20
//! (aarch64-linux-android, script-file probes).

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

fn assert_clean(stderr: &str) {
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
}

const PROBE_PRELUDE: &str = "f() { echo \"argc=$#\"; for x in \"$@\"; do echo \"[$x]\"; done; }\n";

/// The reporter's exact reproducer: the embedded slice splits the whole
/// quoted word with affixes on the first/last element.
#[test]
fn embedded_array_at_slice_splits_quoted_word() {
    let (stdout, stderr, _) = rubash(&format!(
        "{PROBE_PRELUDE}a=(x y z)\nf \"A|${{a[@]:1:2}}|B\""
    ));
    assert_clean(&stderr);
    assert_eq!(stdout, "argc=2\n[A|y]\n[z|B]\n");
}

/// Standalone slice keeps working (no regression).
#[test]
fn standalone_array_at_slice_still_splits() {
    let (stdout, stderr, _) = rubash(&format!("{PROBE_PRELUDE}a=(x y z)\nf \"${{a[@]:1:2}}\""));
    assert_clean(&stderr);
    assert_eq!(stdout, "argc=2\n[y]\n[z]\n");
}

/// Same root cause hits positional slices (issue follow-up comment).
#[test]
fn embedded_positional_at_slice_splits_quoted_word() {
    let (stdout, stderr, _) = rubash(&format!(
        "{PROBE_PRELUDE}set -- p0 p1 p2 p3\nf \"$#|$2|${{@:2:2}}|$*\""
    ));
    assert_clean(&stderr);
    assert_eq!(stdout, "argc=2\n[4|p1|p1]\n[p2|p0 p1 p2 p3]\n");
}

/// Empty elements survive the embedded slice as empty fields, like the
/// unsliced `"${a[@]}"` form.
#[test]
fn embedded_array_at_slice_keeps_empty_elements() {
    let (stdout, stderr, _) = rubash(&format!(
        "{PROBE_PRELUDE}a=(x \"\" z)\nf \"pre${{a[@]:0:3}}post\""
    ));
    assert_clean(&stderr);
    assert_eq!(stdout, "argc=3\n[prex]\n[]\n[zpost]\n");
}

/// Quoted slice under a non-default IFS still splits (elements stay
/// IFS-protected, the slice separator splits).
#[test]
fn embedded_array_at_slice_splits_under_custom_ifs() {
    let (stdout, stderr, _) = rubash(&format!(
        "{PROBE_PRELUDE}a=(x \"y y2\" z)\nIFS=:\nf \"pre${{a[@]:1:2}}post\"\nunset IFS"
    ));
    assert_clean(&stderr);
    assert_eq!(stdout, "argc=2\n[prey y2]\n[zpost]\n");
}

/// Unquoted embedded slice under a non-default IFS fans out through the
/// same path (GNU joins @ with IFS[0] then field-splits).
#[test]
fn unquoted_embedded_array_at_slice_splits_under_custom_ifs() {
    let (stdout, stderr, _) = rubash(&format!(
        "{PROBE_PRELUDE}a=(x \"y y2\" z)\nIFS=:\nf pre${{a[@]:1:2}}post\nunset IFS"
    ));
    assert_clean(&stderr);
    assert_eq!(stdout, "argc=2\n[prey y2]\n[zpost]\n");
}

/// `[*]` slices still join into one word (GNU string_list_dollar_star).
#[test]
fn embedded_array_star_slice_stays_joined() {
    let (stdout, stderr, _) = rubash(&format!(
        "{PROBE_PRELUDE}a=(x \"y y2\" z)\nf \"A|${{a[*]:1:2}}|B\""
    ));
    assert_clean(&stderr);
    assert_eq!(stdout, "argc=1\n[A|y y2 z|B]\n");
}

/// Operator-looking rests are not slices: `:-` stays the default operator.
#[test]
fn colon_operator_forms_are_not_slices() {
    let (stdout, stderr, _) = rubash(&format!(
        "{PROBE_PRELUDE}a=(x \"y y2\" z)\nu=zz\nf \"${{u:-${{a[@]:1:2}}}}\"\nf \"${{nope:-dflt}}\""
    ));
    assert_clean(&stderr);
    assert_eq!(stdout, "argc=1\n[zz]\nargc=1\n[dflt]\n");
}

/// For-list words use the same word-boundary semantics.
#[test]
fn for_list_embedded_array_at_slice_splits() {
    let (stdout, stderr, _) = rubash(&format!(
        "{PROBE_PRELUDE}a=(x \"y y2\" z)\nfor v in \"A|${{a[@]:1:2}}|B\"; do echo \"for=[$v]\"; done"
    ));
    assert_clean(&stderr);
    assert_eq!(stdout, "for=[A|y y2]\nfor=[z|B]\n");
}
