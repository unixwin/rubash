//! Regression pins for rubash#3287 — a fatal `$(( ))` fast assignment
//! leaks the arithmetic error flags into the NEXT command: every
//! following `(( ))` is skipped before evaluation (silent, rc=1)
//! until some simple command's `abort_on_expansion_errors` check
//! happens to consume the flags.
//!
//! Mechanism (see the issue for the full analysis):
//!   - `expand_fast_assignment_value` (assignment_expansion.rs)
//!     sets `arithmetic_fatal_error` and the `arithmetic_expansion_error`
//!     print latch when the RHS `$(( ))` fails fatally. The assignment
//!     is applied AFTER the simple command's abort checks, so the flags
//!     survive the command.
//!   - `eval_arithmetic_command_value_with_flags` (arithmetic/mod.rs)
//!     returns `None` early on a set `arithmetic_fatal_error` —
//!     intended for a NESTED fatal inside this evaluation
//!     (`(( i < $((1/0)) ))`), but a stale flag triggers it too.
//!   - the `None` arm's evalerror report is likewise suppressed by
//!     the stale print latch, so the skip is fully silent.
//!
//! GNU's rule (no cross-command error state exists in bash at all:
//! expr.c evalerror jumps DISCARD for the CURRENT command only —
//! execute_cmd.c:3937 execute_arith_command runs each `(( ))`
//! through a fresh expand_arith_string + evalexp):
//!   - a fatal assignment fails its own command (status 1, its two
//!     diagnostics print once, unlabeled, in evaluation order) and
//!     the NEXT command starts clean — its own evaluation runs and
//!     its own diagnostics print;
//!   - a fatal NESTED expansion inside a `(( ))` still abandons
//!     that evaluation with exactly one diagnostic (the nested
//!     expansion's own line), no second synthesized line.
//!
//! The fix consumes the three flags as ONE error record at the
//! boundary of the command that raised it: the assignment-result
//! boundary (assignment_expansion.rs), the permanent-assignment
//! boundary (temporary_assignments.rs), the `(( ))` command itself
//! (arithmetic_aliases.rs — a fatal nested expansion returns the
//! ExpansionFailure DISCARD), and the `for (( ))` header (the
//! compound-expansion abandon consumer in compound_exec.rs).
//!
//! GNU expectations below are the literal GNU bash 5.3.20 outputs
//! recorded in the issue (canary scripts `seq-test3.sh` /
//! `seq-test4.sh`, head binary vs GNU oracle, same environment) and
//! in tests/issue3246_arith_nameref_eval_order.rs (n1/n3/fatal arms,
//! same diagnostic shapes, captured 2026-11-11). The compound
//! `gnu-extra.sh` 8/8-line acceptance in the issue needs its probe
//! file, which is not present in this workspace — owed, not pinned
//! here; the canary pins below cover the same mechanism.
#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3287-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Base command with the whole locale family removed from the
/// inherited environment so each case starts from a known state.
fn base() -> Command {
    let mut cmd = Command::new(BIN);
    for var in [
        "LANG",
        "LC_ALL",
        "LC_COLLATE",
        "LC_CTYPE",
        "LC_MESSAGES",
        "LC_NUMERIC",
        "LC_TIME",
    ] {
        cmd.env_remove(var);
    }
    cmd.stdin(std::process::Stdio::null());
    cmd
}

fn run_script(tag: &str, body: &str) -> (String, String, String, Option<i32>) {
    let dir = scratch(tag);
    let script = dir.join("probe.sh");
    std::fs::write(&script, body).expect("write script");
    let output = base()
        .arg(&script)
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let result = (
        script.to_string_lossy().into_owned(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn issue3287_canary_fatal_fast_assignment_then_arith_command() {
    // The issue's canary: a fatal fast assignment, then two `(( ))`
    // with only a plain echo between. GNU evaluates both `(( ))`
    // (nameref diag + rc=0 each); the plain echo does NOT clear
    // anything — the leak must not exist in the first place.
    let (path, stdout, stderr, code) = run_script(
        "canary",
        "declare -n ref\nx=$(( ref=1, 1/0 ))\n(( ref=1 ))\necho \"canary1 rc=$?\"\n(( ref=1 ))\necho \"canary2 rc=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "canary1 rc=0\ncanary2 rc=0\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 2: `1': not a valid identifier\n\
             {path}: line 2: ref=1, 1/0 : division by 0 (error token is \"0 \")\n\
             {path}: line 3: ((: `1': not a valid identifier\n\
             {path}: line 5: ((: `1': not a valid identifier\n"
        )
    );
}

#[test]
fn issue3287_echo_with_arith_expansion_between() {
    // The issue's seq-test4 shape: an echo carrying its own fatal
    // `$(( 1/0 ))` between the fatal assignment and the next `(( ))`.
    // The echo is discarded (prints nothing, rc=1); the following
    // `(( ref=1, 1/0 ))` evaluates normally and prints BOTH of its
    // diagnostics (nameref diag, then the div-by-zero line).
    let (path, stdout, stderr, code) = run_script(
        "echo-arith",
        "declare -n ref\nx=$(( ref=1, 1/0 ))\necho $(( 1/0 ))\n(( ref=1, 1/0 ))\necho \"after rc=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after rc=1\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 2: `1': not a valid identifier\n\
             {path}: line 2: ref=1, 1/0 : division by 0 (error token is \"0 \")\n\
             {path}: line 3: 1/0 : division by 0 (error token is \"0 \")\n\
             {path}: line 4: ((: `1': not a valid identifier\n\
             {path}: line 4: ((: ref=1, 1/0 : division by 0 (error token is \"0 \")\n"
        )
    );
}

#[test]
fn issue3287_nested_fatal_inside_arith_command_still_abandons() {
    // The counter-case a snapshot-style fix would break: a fatal
    // nested expansion INSIDE this `(( ))`'s own evaluation must
    // still abandon it — exactly one diagnostic (the nested
    // expansion's own line), no second synthesized line from the
    // `(( ))` None arm. GNU DISCARD (subst.c:4296) abandons the
    // rest of the `(( ))`'s own line (`same` never prints) while
    // the script continues on the next line (`after rc=1`).
    let (path, stdout, stderr, code) = run_script(
        "nested-fatal",
        "(( x = $(( 1/0 )) )); echo \"same rc=$?\"\necho \"after rc=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after rc=1\n");
    assert_eq!(
        stderr,
        format!("{path}: line 1: 1/0 : division by 0 (error token is \"0 \")\n")
    );
}

#[test]
fn issue3287_let_after_fatal_fast_assignment() {
    // `let` takes the same evaluator entry as `(( ))`, so a stale
    // fatal flag would skip it the same way. GNU evaluates it
    // normally (nameref diag with the `let:` label, rc=0).
    let (path, stdout, stderr, code) = run_script(
        "let-after",
        "declare -n ref\nx=$(( ref=1, 1/0 ))\nlet \"ref=1\"\necho \"rc=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 2: `1': not a valid identifier\n\
             {path}: line 2: ref=1, 1/0 : division by 0 (error token is \"0 \")\n\
             {path}: line 3: let: `1': not a valid identifier\n"
        )
    );
}

#[test]
fn issue3287_for_header_after_fatal_fast_assignment() {
    // A `for (( ))` header evaluates through the same entry, so a
    // stale fatal flag would make its test evaluate to None. GNU
    // runs the loop normally after the fatal assignment's own
    // diagnostics.
    let (path, stdout, stderr, code) = run_script(
        "for-after",
        "x=$(( 1/0 ))\nfor (( i=0; i<3; i++ )); do echo \"i=$i\"; done\necho \"done rc=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "i=0\ni=1\ni=2\ndone rc=0\n");
    assert_eq!(
        stderr,
        format!("{path}: line 1: 1/0 : division by 0 (error token is \"0 \")\n")
    );
}

#[test]
fn issue3287_for_header_own_nested_fatal_abandons() {
    // The `for (( ))` header's OWN nested fatal: the init's
    // `$(( 1/0 ))` fails during expansion, so GNU DISCARD
    // (subst.c:4296) abandons the whole `for` command — the body
    // never runs, the same-line echo never prints — and the script
    // continues on the next line with rc=1. The expectation is
    // derived from the DISCARD mechanism (same as the fatal
    // assignment's self-discard pin above), not a direct GNU probe
    // of this shape.
    let (path, stdout, stderr, code) = run_script(
        "for-own-fatal",
        "for (( i=$(( 1/0 )); i<1; i++ )); do echo \"i=$i\"; done; echo \"same=$?\"\necho \"next=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "next=1\n");
    assert_eq!(
        stderr,
        format!("{path}: line 1: 1/0 : division by 0 (error token is \"0 \")\n")
    );
}

#[test]
fn issue3287_declare_integer_assignment_leak() {
    // `declare -i` assignments apply through
    // `apply_permanent_assignments`, which cleared only the print
    // latch — a fatal RHS there leaked the fatal flag into the
    // next `(( ))` (HEAD: `a=1`). GNU runs the next `(( ))`
    // normally (a=0).
    let (path, stdout, stderr, code) = run_script(
        "declare-i",
        "declare -i i\ni=$(( 1/0 ))\n(( 1 )); echo \"a=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "a=0\n");
    assert_eq!(
        stderr,
        format!("{path}: line 2: 1/0 : division by 0 (error token is \"0 \")\n")
    );
}

#[test]
fn issue3287_plain_div_by_zero_repeats() {
    // Control: consecutive failing `(( ))` commands each report
    // their own diagnostic — no latch may suppress the second one.
    let (path, stdout, stderr, code) = run_script(
        "repeat-div0",
        "(( 1/0 )); echo \"a=$?\"\n(( 1/0 )); echo \"b=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "a=1\nb=1\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: ((: 1/0 : division by 0 (error token is \"0 \")\n\
             {path}: line 2: ((: 1/0 : division by 0 (error token is \"0 \")\n"
        )
    );
}

#[test]
fn issue3287_fatal_assignment_itself_still_discards() {
    // The fatal assignment's own command is still discarded (the
    // assignment never lands, rc=1) — the fix must consume the
    // flags in the assignment's own command, not let the next
    // command eat the discard.
    let (path, stdout, stderr, code) = run_script(
        "self-discard",
        "x=$(( 1/0 ))\necho \"after rc=$? x=[$x]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after rc=1 x=[]\n");
    assert_eq!(
        stderr,
        format!("{path}: line 1: 1/0 : division by 0 (error token is \"0 \")\n")
    );
}
