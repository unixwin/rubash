//! rubash#3034 — assignment `y="($x)"` leaks a 0x10 carrier byte plus a
//! pair of `"` into the stored value.
//!
//! Root cause: the compound-claim calls in assignment expansion
//! (`expand_compound_positional_at_assignment` /
//! `expand_unquoted_parameter_compound_assignment`) implement GNU
//! `expand_compound_array_assignment` (arrayfunc.c:557) but were called on
//! ANY parenthesized value, with no check that the word is a genuine
//! compound assignment. A quoted scalar word such as `y="($x)"` — whose
//! parens are literal data inside double quotes — was claimed as an
//! element list, and its products ride the ARRAY_FIELD_SPLIT_MARKER-tagged
//! field transport (`\x10"v1.0"`), which leaked verbatim into scalar
//! storage: `(\x10"v1.0")` instead of `(v1.0)`. The same shape leaked
//! through `y='('$x')'`, argument words (`echo "y=($x)"`), and prefix
//! assignments (`y="($x)" printenv y`). The fix gates both claim calls on
//! the parser's compound decision (COMPOUND_ASSIGNMENT_MARKER, U+E31A):
//! only an unquoted `(` immediately after `=` starts
//! `parse_compound_assignment` (parse.y:5648-5657, parse.y:7104); a
//! quoted scalar takes the assignment-string path (subst.c:4357
//! `expand_string_assignment`).
//!
//! This is the root of #3005: git-prompt.sh's detached-HEAD branch does
//! `b="$(git describe …)"` and then `b="($b)"` — the second step is
//! exactly this shape, so `__git_ps1` rendered ` ((\x10"t2"))` instead of
//! ` ((t2))`. The `git_ps1_detached_chain` cell below pins the
//! assignment-level value `(v1.0)` plus the prompt-shaped rendering
//! ` ((v1.0))`.
//!
//! GNU anchors: parse.y:5648-5657 (compound decision), parse.y:7104
//! (`parse_compound_assignment`), arrayfunc.c:557
//! (`expand_compound_array_assignment`), subst.c:4357
//! (`expand_string_assignment`). Oracle evidence: the byte tables below,
//! GNU bash 5.3.20 vs rubash master 30ddc122.
//!
//! Harness: `rubash -c` via `CARGO_BIN_EXE_rubash`, byte-level stdout
//! assertions. `LC_ALL=C.UTF-8` is set on the child explicitly so the pin
//! is hermetic under any ambient locale.

use std::process::{Command, Stdio};

fn run(script: &str) -> Vec<u8> {
    let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let output = child.wait_with_output().expect("wait rubash");
    assert!(
        output.status.success(),
        "rubash -c {script:?} exited with {}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn expect(script: &str, expected: &[u8], ctx: &str) {
    let out = run(script);
    assert_eq!(out, expected, "{ctx}: got {out:?}, want {expected:?}");
}

#[test]
fn paren_quoted_scalar_param() {
    expect(
        "x=v1.0; y=\"($x)\"; printf '%s' \"$y\"",
        b"(v1.0)",
        "y=\"($x)\" must store (v1.0), no 0x10 carrier or quotes",
    );
}

#[test]
fn paren_quoted_braced_param() {
    expect(
        "x=v1.0; y=\"(${x})\"; printf '%s' \"$y\"",
        b"(v1.0)",
        "y=\"(${x})\" must store (v1.0)",
    );
}

#[test]
fn paren_single_quoted_parens_unquoted_param() {
    expect(
        "x=v1.0; y='('$x')'; printf '%s' \"$y\"",
        b"(v1.0)",
        "y='('$x')' must store (v1.0)",
    );
}

#[test]
fn two_expansions_inside_parens() {
    expect(
        "x=v1.0; y=\"($x $x)\"; printf '%s' \"$y\"",
        b"(v1.0 v1.0)",
        "y=\"($x $x)\" must store (v1.0 v1.0)",
    );
}

#[test]
fn space_value_inside_parens() {
    expect(
        "x=\"a b\"; y=\"($x)\"; printf '%s' \"$y\"",
        b"(a b)",
        "y=\"($x)\" with x='a b' must store (a b), no re-split",
    );
}

#[test]
fn chained_paren_assignment() {
    expect(
        "x=v1.0; y=\"($x)\"; y2=\"($y)\"; printf '%s' \"$y2\"",
        b"((v1.0))",
        "chained y2=\"($y)\" must store ((v1.0)), leak must not nest",
    );
}

#[test]
fn git_ps1_detached_chain_3005() {
    // #3005 root: describe capture then paren wrap; the prompt renders
    // the value inside its own literal ` (...)` format.
    expect(
        "b=\"$(echo v1.0)\"; b=\"($b)\"; printf ' (%s)' \"$b\"",
        b" ((v1.0))",
        "__git_ps1 detached chain must render ((v1.0))",
    );
    expect(
        "b=\"$(echo v1.0)\"; b=\"($b)\"; printf '%s' \"$b\"",
        b"(v1.0)",
        "assignment-level value of the #3005 chain is (v1.0)",
    );
}

#[test]
fn assignment_shaped_argument_word() {
    expect(
        "x=v1.0; echo \"y=($x)\"",
        b"y=(v1.0)\n",
        "echo \"y=($x)\" must print y=(v1.0)",
    );
}

#[test]
fn prefix_assignment_paren_scalar() {
    expect(
        "x=v1.0; y=\"($x)\" printenv y",
        b"(v1.0)\n",
        "prefix assignment y=\"($x)\" must reach the child as (v1.0)",
    );
}

#[test]
fn compound_assignment_still_array() {
    // Guards: genuine compound assignments keep their array semantics.
    expect(
        "x=v1.0; y=($x); printf '%s|%s' \"${y[0]}\" \"${#y[@]}\"",
        b"v1.0|1",
        "y=($x) stays a one-element array",
    );
    expect(
        "x=v1.0; y=(\"$x\"); printf '%s|%s' \"${y[0]}\" \"${#y[@]}\"",
        b"v1.0|1",
        "y=(\"$x\") stays a one-element array",
    );
    expect(
        "x=v1.0; y=(\"$x\" \"$x\"); printf '%s|%s' \"${y[0]} ${y[1]}\" \"${#y[@]}\"",
        b"v1.0 v1.0|2",
        "y=(\"$x\" \"$x\") stays a two-element array",
    );
}

#[test]
fn unbalanced_parens_and_brackets_control() {
    expect(
        "x=v1.0; y=\"($x\"; printf '%s' \"$y\"",
        b"(v1.0",
        "unbalanced y=\"($x\" stores (v1.0 (never regressed)",
    );
    expect(
        "x=v1.0; y=\"$x)\"; printf '%s' \"$y\"",
        b"v1.0)",
        "unbalanced y=\"$x)\" stores v1.0) (never regressed)",
    );
    expect(
        "x=v1.0; y=\"[$x]\"; printf '%s' \"$y\"",
        b"[v1.0]",
        "y=\"[$x]\" stores [v1.0] (never regressed)",
    );
}

#[test]
fn eval_paren_scalar() {
    expect(
        "x=v1.0; eval 'y=\"($x)\"'; printf '%s' \"$y\"",
        b"(v1.0)",
        "eval'd y=\"($x)\" must store (v1.0)",
    );
}
