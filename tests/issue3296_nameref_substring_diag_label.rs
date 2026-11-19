//! rubash#3296 — substring-offset/length expansion through a nameref must
//! carry the variable-name label on its diagnostic, exactly once, at the
//! right line — GNU prefixes the name, rubash printed it unlabeled (and
//! for `[@]`/`@` in command position, two/three times on the next line, or
//! not at all in a one-shot `-c` script).
//!
//! GNU anchors (bash 5.3.20, verified by probe against the oracle):
//! - `parameter_brace_substring` (subst.c:9050-9075) sets
//!   `this_command_name = varname` ("v", "a[0]", "a[@]", "a[*]", "@")
//!   around `verify_substring_values` -> `evalexp` (subst.c:8395).
//! - The offset `ref=1` binds through the invisible nameref
//!   (`declare -n ref` with no target): `bind_variable_internal`
//!   (variables.c:3078) calls `sh_invalidid("1")` (the RHS fails
//!   `valid_nameref_value`), which reaches `builtin_error`
//!   (builtins/common.c:208-211) whose prolog (builtins/common.c:83-95)
//!   prints `<script>: line N: <this_command_name>: ` before the message.
//! - A `$(( ... ))` operand is expanded first by `expand_arith_string`,
//!   whose arithmetic arm NULLs `this_command_name` around `evalexp`
//!   ("No error messages", subst.c:10873-10878), so the same bind error
//!   there carries NO label.
//! - The bind failure is non-fatal: the assignment expression still yields
//!   the RHS value (`$(( ref=1 ))` is 1), so `${v:ref=1:2}` slices `bc`.
//!
//! Rubash mechanism (tip 61712935): the offset/length arithmetic runs
//! through the arithmetic event log (`ArithEvalEvent::Diag`), but the
//! substring parse sites never drain it with the varname label. The
//! command pre-scan (`validate_command_parameter_expansions`,
//! command_prepare.rs:1031) plus the `[@]`/`@` word-shape probes
//! (`array_at_word_values`, arrays/executor.rs:1189;
//! `quoted_positional_at_word_values`,
//! command_substitution_values.rs:368) re-evaluate the offset 2-3 times
//! via the `&self` free-function path, recording one event per parse;
//! the `&mut` evaluation (the only one that drains) clears those stale
//! events for scalar/`[*]`/member shapes but never runs for `[@]`/`@`
//! in command position, so their events drain at the next command
//! boundary — unlabeled, duplicated, on the wrong line (or never, in a
//! one-shot `-c` script).
//!
//! Expectations below are byte-measured against GNU bash 5.3.20.
//! Harness mirrors issue2713_substring_arith_diagnostic_raw_text:
//! `rubash -c` via `CARGO_BIN_EXE_rubash`, `LC_ALL=C.UTF-8` pinned.

use std::process::{Command, Stdio};

struct Outcome {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    code: i32,
}

fn run(script: &str) -> Outcome {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    Outcome {
        stdout: output.stdout,
        stderr: output.stderr,
        code: output.status.code().unwrap_or(-1),
    }
}

/// Non-fatal expansion diagnostic: rc 0, expected stdout, and the labeled
/// diagnostic exactly `times` times on stderr.
fn expect_diag_times(script: &str, needle: &str, times: usize, stdout: &[u8], ctx: &str) {
    let out = run(script);
    assert_eq!(
        out.code,
        0,
        "{ctx}: rc must be 0 (non-fatal diagnostic); stdout: {:?} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        out.stdout,
        stdout,
        "{ctx}: stdout must be {stdout:?}; got {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    let count = stderr.matches(needle).count();
    assert_eq!(
        count, times,
        "{ctx}: stderr must contain {needle:?} exactly {times} time(s); got {count}; stderr: {stderr}"
    );
}

#[test]
fn scalar_offset_carries_varname_label_once() {
    // GNU: `v: `1': not a valid identifier`, once; slice is `bc`.
    expect_diag_times(
        "declare -n ref; v=abcdef; echo \"${v:ref=1:2}\"",
        "v: `1': not a valid identifier",
        1,
        b"bc\n",
        "A1 scalar offset",
    );
}

#[test]
fn scalar_offset_label_echoes_rhs_value() {
    // The quoted token is the RHS being bound through the nameref.
    expect_diag_times(
        "declare -n ref2; v=abcdef; echo \"${v:ref2=5:2}\"",
        "v: `5': not a valid identifier",
        1,
        b"f\n",
        "A2 scalar offset, rhs 5",
    );
}

#[test]
fn scalar_length_operand_carries_label_once() {
    // The label also covers the length operand (same verify_substring_values
    // call, same this_command_name window). The nameref must be declared:
    // without `declare -n ref`, GNU binds `ref=1` silently and prints no
    // diagnostic (oracle-checked 2026-11-19); the original pin dropped
    // the declaration and asserted a diagnostic GNU never emits.
    expect_diag_times(
        "declare -n ref; v=abcdef; echo \"${v:1:ref=1}\"",
        "v: `1': not a valid identifier",
        1,
        b"b\n",
        "A3 scalar length operand",
    );
}

#[test]
fn array_member_label_is_name_plus_subscript() {
    // GNU varname is the name as written: `a[0]`.
    expect_diag_times(
        "declare -n ref; a=(a b c); echo \"${a[0]:ref=1:2}\"",
        "a[0]: `1': not a valid identifier",
        1,
        b"\n",
        "A4 array member",
    );
}

#[test]
fn array_at_quoted_carries_label_once() {
    // The `[@]` command-position shape: tip printed the diagnostic twice,
    // unlabeled, on the next line (and never in a one-shot `-c` script).
    expect_diag_times(
        "declare -n ref; a=(a b c); echo \"${a[@]:ref=1}\"",
        "a[@]: `1': not a valid identifier",
        1,
        b"b c\n",
        "A5 array @ quoted",
    );
}

#[test]
fn array_at_unquoted_carries_label_once() {
    expect_diag_times(
        "declare -n ref; a=(a b c); echo ${a[@]:ref=1}",
        "a[@]: `1': not a valid identifier",
        1,
        b"b c\n",
        "A6 array @ unquoted",
    );
}

#[test]
fn positional_at_carries_label_once() {
    // The `@` command-position shape: tip printed the diagnostic three
    // times, unlabeled, on the next line.
    expect_diag_times(
        "declare -n ref; echo \"${@:ref=1}\"",
        "@: `1': not a valid identifier",
        1,
        b"\n",
        "A7 positional @",
    );
}

#[test]
fn array_star_carries_label_once() {
    expect_diag_times(
        "declare -n ref; a=(a b c); echo \"${a[*]:ref=1}\"",
        "a[*]: `1': not a valid identifier",
        1,
        b"b c\n",
        "A8 array *",
    );
}

#[test]
fn dollar_arith_offset_carries_no_label() {
    // Control: a `$(( ... ))` operand is expanded with this_command_name
    // NULL (subst.c:10873), so the same bind error stays unlabeled.
    // A fix that labels every substring diagnostic would break this.
    let out = run("declare -n ref; v=abcdef; echo \"${v:$(( ref=1 )):2}\"");
    assert_eq!(out.code, 0, "A9: rc must be 0");
    assert_eq!(out.stdout, b"bc\n", "A9: stdout must be bc");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        stderr.matches("`1': not a valid identifier").count(),
        1,
        "A9: unlabeled diagnostic exactly once; got {stderr}"
    );
    assert!(
        !stderr.contains("v: `1'"),
        "A9: must NOT gain a v: label; got {stderr}"
    );
}

#[test]
fn invalid_element_label_echoes_element_spelling() {
    // `a[]` as the offset lvalue: the token is the element spelling.
    expect_diag_times(
        "v=abcdef; echo \"${v:a[]=1:2}\"",
        "v: `a[]': not a valid identifier",
        1,
        b"bc\n",
        "A10 invalid a[] element",
    );
}

#[test]
fn two_identical_expansions_print_twice() {
    // GNU prints one diagnostic per expansion. Guards against a
    // deduplication-style fix collapsing genuinely repeated diagnostics.
    expect_diag_times(
        "declare -n ref; v=abcdef; echo \"${v:ref=1:2} ${v:ref=1:2}\"",
        "v: `1': not a valid identifier",
        2,
        b"bc bc\n",
        "A11 two identical expansions",
    );
}

#[test]
fn assignment_context_carries_label_once() {
    expect_diag_times(
        "declare -n ref; v=abcdef; x=\"${v:ref=1:2}\"; echo \"$x\"",
        "v: `1': not a valid identifier",
        1,
        b"bc\n",
        "A12 assignment RHS",
    );
}

#[test]
fn function_body_carries_label_once() {
    expect_diag_times(
        "declare -n ref; v=abcdef; f() { echo \"${v:ref=1:2}\"; }; f",
        "v: `1': not a valid identifier",
        1,
        b"bc\n",
        "A13 inside a function",
    );
}

#[test]
fn dollar_arith_length_carries_no_label() {
    // Same no-label rule for the length operand.
    let out = run("declare -n ref; v=abcdef; echo \"${v:1:$(( ref=1 ))}\"");
    assert_eq!(out.code, 0, "A14: rc must be 0");
    assert_eq!(out.stdout, b"b\n", "A14: stdout must be b");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        stderr.matches("`1': not a valid identifier").count(),
        1,
        "A14: unlabeled diagnostic exactly once; got {stderr}"
    );
    assert!(
        !stderr.contains("v: `1'"),
        "A14: must NOT gain a v: label; got {stderr}"
    );
}
