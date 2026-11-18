//! rubash#2648 — a parenthesised operand after `<`/`>` inside `$(( ))`
//! was additionally executed as a command: the raw-word process-
//! substitution scan treated `>(` / `<(` inside an arithmetic span as a
//! process substitution and ran its body (`2+3: command not found`).
//!
//! GNU anchor: subst.c:10915/10926 (expand_word_internal's `$` arm) —
//! a `$((...))` span is extracted whole by extract_arithmetic_subst and
//! expanded by expand_arith_string, so its characters never reach the
//! `<`/`>` arms (subst.c:11365-11378) that extract process
//! substitutions. Only a `<(`/`>(` at the word's own scan level is a
//! process substitution.
//!
//! Harness: `rubash -c` via `CARGO_BIN_EXE_rubash`, byte-level stdout
//! assertions plus exit code and stderr assertions. `LC_ALL=C.UTF-8`
//! is set on the child explicitly so the pin is hermetic.

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

/// The arithmetic value is computed and NOTHING is executed: stdout is
/// exactly the expected text, rc 0, stderr empty.
fn expect_value(script: &str, expected: &[u8], ctx: &str) {
    let out = run(script);
    assert_eq!(
        out.stdout,
        expected,
        "{ctx}: stdout mismatch; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        out.code,
        0,
        "{ctx}: rc must be 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stderr.is_empty(),
        "{ctx}: stderr must be empty (nothing may execute); got {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn greater_than_paren_operand_is_not_a_process_substitution() {
    expect_value("echo $(( 4>(2+3) ? 1 : 32))", b"32\n", "4>(2+3)");
}

#[test]
fn less_than_paren_operand_is_not_a_process_substitution() {
    expect_value("echo $(( 4<(2+3) ? 1 : 32))", b"1\n", "4<(2+3)");
}

#[test]
fn parenthesised_operands_on_both_sides() {
    expect_value("echo $(( (2+2)>(2+3) ? 2**0 : 32))", b"32\n", "(2+2)>(2+3)");
}

#[test]
fn nested_paren_operands_sum() {
    expect_value(
        "echo $(( (4>(2+3)) + (4<(2+3)) ))",
        b"1\n",
        "(4>(2+3)) + (4<(2+3))",
    );
}

#[test]
fn paren_operand_inside_subscript() {
    expect_value(
        "declare -a a=(10 20 30); echo $(( a[4>(2+3) ? 0 : 1] ))",
        b"20\n",
        "a[4>(2+3) ? 0 : 1]",
    );
}

#[test]
fn paren_operand_inside_command_substitution() {
    expect_value(
        "echo $(echo $(( 4>(2+3) ? 1 : 32 )))",
        b"32\n",
        "$(... $(( 4>(2+3) ... )) ...)",
    );
}

#[test]
fn assignment_form_does_not_materialise_a_procsub_fd() {
    // On the defective tree the substitution was materialised and its
    // /dev/fd/N path spliced into the arithmetic text, which then
    // failed with a spurious `division by 0` diagnostic and rc 1.
    expect_value(
        "x=$(( 4>(2+3) ? 1 : 32)); echo $x",
        b"32\n",
        "x=$(( 4>(2+3) ? 1 : 32))",
    );
}

#[test]
fn arithmetic_command_form() {
    expect_value(
        "(( 4>(2+3) ? (x=1) : (x=32) )); echo $x",
        b"32\n",
        "(( 4>(2+3) ? (x=1) : (x=32) ))",
    );
}

#[test]
fn real_process_substitution_still_runs() {
    expect_value("cat <(echo hi)", b"hi\n", "cat <(echo hi)");
}

#[test]
fn real_process_substitution_as_redirect_source() {
    expect_value(
        "read v < <(echo hi); echo $v",
        b"hi\n",
        "read v < <(echo hi)",
    );
}

#[test]
fn greater_equal_is_not_affected() {
    expect_value("echo $(( 4>=(2+3) ? 1 : 32 ))", b"32\n", "4>=(2+3)");
}
