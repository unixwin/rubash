//! rubash#3251 — `set -u` is silent for a bare `$`-expansion inside an
//! arithmetic command: `(( $undef ))` must report `undef: unbound
//! variable` with rc 127, as GNU does. On the defective tree the `$undef`
//! is expanded away to empty before the nounset scan ever runs, so the
//! command falls through silently (rc 0). Same family as rubash#3199
//! (pre-expansion swallows the name), but outside #3199's subscript
//! face: #3199's fix scans subscript text only, and a bare top-level
//! `$name` never reaches it.
//!
//! GNU anchors (bash 5.3, oracle binary probed 2026-11-12):
//! - execute_cmd.c:3936: `(( ))` expands via expand_arith_string, then
//!   evalexp; execute_cmd.c:3171: `$(( ))` is the same via
//!   eval_arith_for_expr.
//! - subst.c:3981 expand_arith_string: the expansion traps an unbound
//!   `$name` read at its `unbound_variable:` label (subst.c:11023-11027
//!   -> err_unboundvar), so the error aborts a non-interactive script
//!   (rc 127, no trailing command runs).
//! - subst.c:10500-10512 param_expand: a plain `$N` positional names
//!   itself `$N`-style in the diagnostic (uerror = "$" + digit), while
//!   a braced `${N}` read names the bare digit.
//! - expr.c:1190-1216 expr_streval: the same trap at evaluation time for
//!   bare names.
//!
//! Harness: `rubash -c` via `CARGO_BIN_EXE_rubash`, `LC_ALL=C.UTF-8`,
//! following the adjacent #3199 pin. stderr is matched by substring
//! (`{name}: unbound variable`) so the shell's error prefix stays free.
//!
//! Scope note — cells deliberately NOT nailed here (measured red on the
//! head tree, separate mechanisms, residuals for follow-up filing):
//! - `let '$undef'` (probe c12): `let` operands arrive already
//!   expanded; GNU reports an arithmetic syntax error, rubash traps
//!   nounset — a `let`-operand face, not this scan.
//! - `${undef:?}` / `${undef:?custom}` (probe c14/c26): the `:?`
//!   error operator is not honoured by the arith expansion walk
//!   (c26 goes silent; c14 names the word `custom` instead of `undef`).
//! - `b['$undef']` (probe q02): GNU reports an operand error, rubash
//!   traps nounset — quoted-subscript-key handling in evaluation.
//! - `$(( i++ )) + b[$undef]` (probe x02): GNU traps the bare `i`
//!   (expr_streval nounset); rubash reports a syntax error.
//!
//! Verification status: the c01–c27/q01–q15/x01 cells below were measured
//! green on the head tree by the lane's GNU three-way probe
//! (`probe-head.log`, 2026-11-12); the nested-`$(( ))` cell c13 is the
//! rework fix in this wave and its base fail-first is inferred from the
//! same erasure mechanism (base c01 red, measured). All cells await the
//! captain's double-sided re-run (UNVERIFIED-BY-CAPTAIN).

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

/// GNU: the nounset error aborts the non-interactive script — no
/// stdout at all, exit 127, and `{name}: unbound variable` on stderr.
fn expect_unbound(script: &str, name: &str, ctx: &str) {
    let out = run(script);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let needle = format!("{name}: unbound variable");
    assert_eq!(
        out.stdout,
        b"",
        "{ctx}: GNU aborts the script on the nounset error, so there must be no stdout; stderr: {stderr}"
    );
    assert_eq!(
        out.code, 127,
        "{ctx}: script must exit 127; stderr: {stderr}"
    );
    assert!(
        stderr.contains(&needle),
        "{ctx}: stderr must contain {needle:?}; got {stderr}"
    );
}

fn expect_stdout(script: &str, expected_stdout: &[u8], ctx: &str) {
    let out = run(script);
    assert_eq!(
        out.stdout,
        expected_stdout,
        "{ctx}: stdout mismatch; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        out.code,
        0,
        "{ctx}: script must exit 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn arith_command_bare_dollar_unbound() {
    expect_unbound("set -u; (( $undef )); echo AFTER", "undef", "(( $undef ))");
}

#[test]
fn arith_expansion_bare_dollar_unbound() {
    expect_unbound(
        "set -u; echo $(( $undef )); echo AFTER",
        "undef",
        "$(( $undef ))",
    );
}

#[test]
fn arith_command_bare_dollar_unbound_plus() {
    expect_unbound(
        "set -u; (( $undef + 1 )); echo AFTER",
        "undef",
        "(( $undef + 1 ))",
    );
}

#[test]
fn arith_command_braced_dollar_unbound() {
    expect_unbound(
        "set -u; (( ${undef} )); echo AFTER",
        "undef",
        "(( ${undef} ))",
    );
}

#[test]
fn arith_command_two_unbound_first_wins() {
    expect_unbound(
        "set -u; (( $undef1 + $undef2 )); echo AFTER",
        "undef1",
        "(( $undef1 + $undef2 ))",
    );
}

#[test]
fn arith_command_assign_rhs_dollar_unbound() {
    expect_unbound(
        "set -u; (( x=$undef )); echo AFTER",
        "undef",
        "(( x=$undef ))",
    );
}

#[test]
fn arith_command_positional_dollar_unbound() {
    expect_unbound("set -u; (( $1 )); echo AFTER", "$1", "(( $1 ))");
}

#[test]
fn arith_command_cmdsubst_plus_dollar_unbound() {
    expect_unbound(
        "set -u; (( $(echo 1) + $undef )); echo AFTER",
        "undef",
        "(( $(echo 1) + $undef ))",
    );
}

#[test]
fn arith_command_guarded_word_dollar_unbound() {
    expect_unbound(
        "set -u; (( ${undef:-$undef2} )); echo AFTER",
        "undef2",
        "(( ${undef:-$undef2} ))",
    );
}

#[test]
fn arith_command_dollar_unbound_aborts_script() {
    expect_unbound(
        "set -u; (( $undef )); echo AFTER; echo SECOND",
        "undef",
        "(( $undef )) aborts before AFTER/SECOND",
    );
}

#[test]
fn arith_command_leading_unbound_beats_subscript() {
    expect_unbound(
        "set -u; (( $undef2 + b[$undef] )); echo AFTER",
        "undef2",
        "(( $undef2 + b[$undef] ))",
    );
}

#[test]
fn arith_command_subscript_unbound_beats_trailing() {
    // GNU names the subscript's `$undef` here, not the trailing
    // `$undef2`: expansion walks left to right and the subscript
    // expands before the trailing read is reached.
    expect_unbound(
        "set -u; b=(x y); (( b[$undef] + $undef2 )); echo AFTER",
        "undef",
        "(( b[$undef] + $undef2 ))",
    );
}

#[test]
fn arith_expansion_dollar_plus_subscript_unbound() {
    expect_unbound(
        "set -u; echo $(( $undef + b[0] )); echo AFTER",
        "undef",
        "$(( $undef + b[0] ))",
    );
}

#[test]
fn arith_command_ternary_dollar_unbound() {
    expect_unbound(
        "set -u; (( $undef ? 1 : 2 )); echo AFTER",
        "undef",
        "(( $undef ? 1 : 2 ))",
    );
}

#[test]
fn arith_command_nested_arith_dollar_unbound() {
    // Rework cell (c13): a nested `$(( ))` inside `(( ))` is expanded
    // inline by the enclosing expansion, not by a separate entry, so
    // the raw-text scan must recurse into it — the old skip of the
    // whole `$(...)` span let this go silent (rc 0).
    expect_unbound(
        "set -u; (( $(( $undef )) )); echo AFTER",
        "undef",
        "(( $(( $undef )) ))",
    );
}

#[test]
fn arith_command_nested_arith_leading_wins() {
    // Leftmost-first across the nest: the leading `$undef2` is
    // reported, not the `$undef` inside the nested `$(( ))` (x01).
    expect_unbound(
        "set -u; (( $undef2 + $(( $undef )) )); echo AFTER",
        "undef2",
        "(( $undef2 + $(( $undef )) ))",
    );
}

#[test]
fn arith_command_single_quoted_dollar_unbound() {
    // A single quote at the top level is data in arith text (GNU
    // expands as if double-quoted), so the `$` read inside it traps.
    expect_unbound(
        "set -u; (( '$undef' )); echo AFTER",
        "undef",
        "(( '$undef' ))",
    );
}

#[test]
fn arith_command_double_quoted_dollar_unbound() {
    expect_unbound(
        "set -u; (( \"$undef\" )); echo AFTER",
        "undef",
        "(( \"$undef\" ))",
    );
}

#[test]
fn arith_command_subscript_positional_unbound() {
    expect_unbound(
        "set -u; b=(x y); (( b[$1] )); echo AFTER",
        "$1",
        "(( b[$1] ))",
    );
}

#[test]
fn arith_command_braced_positional_unbound() {
    // Braced `${1}` names the bare digit, unlike plain `$1` (q07).
    expect_unbound("set -u; (( ${1} )); echo AFTER", "1", "(( ${1} ))");
}

#[test]
fn arith_expansion_positional_unbound() {
    expect_unbound("set -u; echo $(( $1 )); echo AFTER", "$1", "$(( $1 ))");
}

#[test]
fn arith_command_subscript_double_quoted_dollar_unbound() {
    // Double quotes never protect a `$` read, inside a subscript or
    // outside it (q11).
    expect_unbound(
        "set -u; b=(x y); (( b[\"$undef\"] )); echo AFTER",
        "undef",
        "(( b[\"$undef\"] ))",
    );
}

#[test]
fn arith_expansion_subscript_positional_unbound() {
    expect_unbound(
        "set -u; echo $(( b[$1] )); echo AFTER",
        "$1",
        "$(( b[$1] ))",
    );
}

#[test]
fn arith_command_guarded_word_positional_unbound() {
    expect_unbound(
        "set -u; (( ${undef:-$1} )); echo AFTER",
        "$1",
        "(( ${undef:-$1} ))",
    );
}

#[test]
fn arith_command_assoc_subscript_dollar_unbound() {
    expect_unbound(
        "set -u; declare -A A; A[x]=1; (( A[$undef] )); echo AFTER",
        "undef",
        "(( A[$undef] ))",
    );
}

#[test]
fn arith_command_assoc_subscript_dollar_unbound_empty_array() {
    expect_unbound(
        "set -u; declare -A A; (( A[$undef] )); echo AFTER",
        "undef",
        "(( A[$undef] )) on a declared-but-empty assoc array",
    );
}

#[test]
fn bound_dollar_evaluates() {
    expect_stdout(
        "set -u; undef=3; (( $undef )); echo AFTER rc=$?",
        b"AFTER rc=0\n",
        "bound $undef",
    );
}

#[test]
fn guarded_default_dollar_evaluates() {
    expect_stdout(
        "set -u; (( ${undef:-5} )); echo AFTER rc=$?",
        b"AFTER rc=0\n",
        "${undef:-5} guard",
    );
}

#[test]
fn dynamic_random_bound() {
    expect_stdout(
        "set -u; (( $RANDOM )); echo AFTER rc=$?",
        b"AFTER rc=0\n",
        "$RANDOM is always bound",
    );
}

#[test]
fn positional_count_bound() {
    // `$#` is 0 with no positionals, so `(( 0 ))` is false (rc 1)
    // but the script continues and exits 0.
    expect_stdout(
        "set -u; (( $# )); echo AFTER rc=$?",
        b"AFTER rc=1\n",
        "$# is always bound",
    );
}

#[test]
fn assoc_bare_key_evaluates() {
    // A bare unbound name as an assoc key evaluates to 0 (GNU:
    // `(( A[k] ))` is false, rc 1) — it is a bare-name read, not a
    // `$` read, so nounset does not trap it here.
    expect_stdout(
        "set -u; declare -A A; A[x]=1; (( A[k] )); echo AFTER rc=$?",
        b"AFTER rc=1\n",
        "bare assoc key A[k]",
    );
}

#[test]
fn no_nounset_dollar_silent() {
    // Without `set -u` the bare read stays silent and `(( ))` is
    // false (rc 1) — the fix must not trap outside nounset.
    expect_stdout("(( $undef )); echo rc=$?", b"rc=1\n", "no set -u control");
}
