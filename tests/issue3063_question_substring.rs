//! rubash#3063 — `${?}` substring expansion returned empty instead of
//! slicing the `$?` value, and bad offsets expanded empty silently
//! instead of reporting GNU's arithmetic error.
//!
//! GNU anchor: in `param_expand` (subst.c) the parameter's value is
//! fetched generically BEFORE the substring dispatch — for `?` that is
//! `itos (last_command_exit_value)` (cf. subst.c:8280) — and passed to
//! `parameter_brace_substring` (subst.c:9050, call site subst.c:10179).
//! The value of `$?` is never NULL, so the subst.c:9057 NULL early
//! return (the rubash#3024 unset short-circuit) never applies and
//! `verify_substring_values` (subst.c:8395) always evaluates the
//! offset/length arithmetic: garbage operands are an expansion error.
//! The same generic fetch serves every special parameter (`$`, `-`,
//! `#`, `!`, `0`, positional digits) in every word context.
//!
//! Base-tree root cause (two layers, one mechanism — the substring
//! machinery never consulted the generic special-parameter fetch):
//!   1. `split_top_level_colon` counted the `?` that IS the parameter
//!      name as a ternary marker (the subst.c:2254-2264 skip_to_delim
//!      rule applies to the substring arithmetic text AFTER the name
//!      has been extracted), so `?:0:1` split as name `?:0` and the
//!      parse died at the name gate — offsets were never evaluated.
//!   2. `expand_braced_substring_parameter` did not resolve special
//!      parameter names itself; the quoted/assignment call sites
//!      fetched the value for it, but the plain-word dispatcher path
//!      (expand_braced_indexed_parameter) did not, so the whole family
//!      sliced empty unquoted. A bare `!` name was likewise never
//!      admitted by the substring parse.
//!
//! Harness: `rubash -c` via `CARGO_BIN_EXE_rubash`, byte-level stdout
//! assertions plus exit code and stderr-substring assertions.
//! `LC_ALL=C.UTF-8` is set on the child explicitly so the pin is
//! hermetic under any ambient locale. Pid-valued cells assert
//! self-consistency against the same shell's `$!`/`$$` slice instead
//! of literal digits.

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

/// Exact stdout, rc 0, empty stderr.
fn expect(script: &str, stdout: &[u8], ctx: &str) {
    let out = run(script);
    assert_eq!(
        out.stdout,
        stdout,
        "{ctx}: stdout mismatch; got {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(
        out.code,
        0,
        "{ctx}: rc must be 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stderr.is_empty(),
        "{ctx}: stderr must be empty; got {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Bad offsets on `$?`: rc 1 with GNU's arithmetic diagnostic.
fn expect_arith_error(script: &str, needle: &str, ctx: &str) {
    let out = run(script);
    assert_eq!(
        out.code,
        1,
        "{ctx}: rc must be 1; stdout: {:?} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(needle),
        "{ctx}: stderr must contain {needle:?}; got {stderr}"
    );
}

#[test]
fn question_substring_slices_status() {
    expect(
        "false; echo \"X${?:0:1}Y\"",
        b"X1Y\n",
        "repro: ${?:0:1} after false",
    );
}

#[test]
fn question_substring_multi_digit_status() {
    expect(
        "( exit 42 ); echo \"X${?:0:2}Y\"",
        b"X42Y\n",
        "${?:0:2} of status 42",
    );
}

#[test]
fn question_substring_offset_only() {
    expect(
        "( exit 42 ); echo \"X${?:1}Y\"",
        b"X2Y\n",
        "${?:1} of status 42",
    );
}

#[test]
fn question_substring_negative_offset() {
    expect(
        "( exit 42 ); echo \"X${?: -2:1}Y\"",
        b"X4Y\n",
        "${?: -2:1} of status 42 (space form: substring, not :- op)",
    );
    expect(
        "false; echo \"X${?: -1:1}Y\"",
        b"X1Y\n",
        "${?: -1:1} of status 1",
    );
}

#[test]
fn question_substring_negative_length() {
    expect(
        "( exit 42 ); echo \"X${?:0:-1}Y\"",
        b"X4Y\n",
        "${?:0:-1} of status 42 drops the last char",
    );
}

#[test]
fn question_substring_offset_past_end_is_empty() {
    expect(
        "false; echo \"X${?:5:2}Y\"",
        b"XY\n",
        "${?:5:2} past the end expands empty, rc 0",
    );
}

#[test]
fn question_substring_unquoted() {
    expect(
        "false; echo X${?:0:1}Y",
        b"X1Y\n",
        "unquoted ${?:0:1} (plain-word dispatcher path)",
    );
}

#[test]
fn question_substring_assignment_rhs() {
    expect(
        "false; v=${?:0:1}; echo \"X${v}Y\"",
        b"X1Y\n",
        "assignment RHS v=${?:0:1}",
    );
}

#[test]
fn question_substring_inside_comsub() {
    expect(
        "echo \"X$(false; echo ${?:0:1})Y\"",
        b"X1Y\n",
        "${?:0:1} inside a command substitution sees the inner status",
    );
}

#[test]
fn question_bad_offsets_error() {
    expect_arith_error(
        "false; echo \"X${?:t5:h2:l}Y\"",
        "?: h2:l: arithmetic syntax error in expression (error token is \":l\")",
        "issue repro: ${?:t5:h2:l} must error like GNU",
    );
}

#[test]
fn question_bad_length_error() {
    expect_arith_error(
        "false; echo \"X${?:0:h2:l}Y\"",
        "?: h2:l: arithmetic syntax error in expression (error token is \":l\")",
        "${?:0:h2:l}: bad length must error like GNU",
    );
}

#[test]
fn question_bad_offsets_error_unquoted() {
    expect_arith_error(
        "false; echo X${?:t5:h2:l}Y",
        "?: h2:l: arithmetic syntax error in expression (error token is \":l\")",
        "unquoted ${?:t5:h2:l} must error like GNU",
    );
}

// ---- Family census: the plain-word (unquoted) path sliced empty for
// ---- every special parameter on the base tree, not just `?`.

#[test]
fn family_unquoted_positional_slices() {
    expect(
        "set -- alpha beta; echo X${1:0:2}Y",
        b"XalY\n",
        "unquoted ${1:0:2}",
    );
}

#[test]
fn family_unquoted_count_slices() {
    expect(
        "set -- a b c; echo X${#:0:1}Y",
        b"X3Y\n",
        "unquoted ${#:0:1}",
    );
}

#[test]
fn family_unquoted_dollar_pid_slices() {
    // The expansion under test must sit in a genuinely unquoted word:
    // compare its slice (captured from an unquoted echo inside a comsub)
    // against the named-variable slice of the same pid.
    expect(
        "p=$$; u=$(echo X${$:0:2}Y); q=$(echo \"X${p:0:2}Y\"); [ \"$u\" = \"$q\" ] && echo OK",
        b"OK\n",
        "unquoted ${$:0:2} equals the $$ slice",
    );
}

#[test]
fn family_unquoted_zero_slices() {
    expect(
        "z=$0; u=$(echo X${0:0:2}Y); q=$(echo \"X${z:0:2}Y\"); [ \"$u\" = \"$q\" ] && echo OK",
        b"OK\n",
        "unquoted ${0:0:2} equals the $0 slice",
    );
}

#[test]
fn family_unquoted_flags_slice() {
    expect(
        "f=$-; u=$(echo X${-:0:2}Y); q=$(echo \"X${f:0:2}Y\"); [ \"$u\" = \"$q\" ] && echo OK",
        b"OK\n",
        "unquoted ${-:0:2} equals the $- slice",
    );
}

#[test]
fn family_bang_slices_when_set() {
    expect(
        "sleep 0.2 & p=$!; [ \"${!:0:2}\" = \"${p:0:2}\" ] && echo OK; wait",
        b"OK\n",
        "quoted ${!:0:2} equals the $! slice",
    );
    expect(
        "sleep 0.2 & p=$!; [ ${!:0:2} = ${p:0:2} ] && echo OK; wait",
        b"OK\n",
        "unquoted ${!:0:2} equals the $! slice",
    );
}

// ---- Controls: shapes that already matched GNU on the base tree and
// ---- must not move.

#[test]
fn control_quoted_family_slices() {
    expect(
        "set -- alpha beta; echo \"X${1:0:2}Y\"",
        b"XalY\n",
        "control: quoted ${1:0:2} already sliced on the base tree",
    );
    expect(
        "set -- a b c; echo \"X${#:0:1}Y\"",
        b"X3Y\n",
        "control: quoted ${#:0:1} already sliced on the base tree",
    );
}

#[test]
fn control_bang_unset_bad_offsets_short_circuit() {
    // `$!` is unset until the first background job, so GNU returns NULL
    // before evaluating the offsets (subst.c:9057): empty, rc 0, silent
    // — the rubash#3024 short-circuit shape.
    expect(
        "echo \"X${!:t5:h2:l}Y\"",
        b"XY\n",
        "control: unset $! with bad offsets stays empty and silent",
    );
}

#[test]
fn control_question_length_form() {
    expect(
        "false; echo \"X${#?}Y\"",
        b"X1Y\n",
        "control: ${#?} length of the status",
    );
}

#[test]
fn control_ternary_offset_still_splits_after_ternary_colon() {
    // The split_top_level_colon ternary rule (subst.c:2254-2264) is
    // load-bearing for real arithmetic offsets: in `${v:j?1:0:j}` the
    // colon after `?` belongs to the ternary, not the slice.
    expect(
        "v=hello; j=1; echo \"X${v:j?1:0:2}Y\"",
        b"XelY\n",
        "control: ternary offset ${v:j?1:0:2} keeps its shape",
    );
}
