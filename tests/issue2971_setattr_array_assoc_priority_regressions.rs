//! rubash#2971: `export -aA name=value` / `readonly -aA name=value`
//! must bind an INDEXED array, not an associative one.
//!
//! GNU mechanism: builtins/setattr.def set_or_show_attributes delegates
//! an assignment operand with arrays_only/assoc_only to a nested
//! declare_builtin with ONE synthesized type letter — `a` when
//! arrays_only, else `A` (setattr.def:240-258) — so for a scalar operand
//! the indexed request wins regardless of option order. The delegation
//! runs only for assignment operands (`if (assign)`): a name-only
//! `export -A x` merely sets att_exported and leaves x's shape alone.
//! Compound operands are the exception: they were already expanded
//! under the assoc interpretation when -A was given, and stay assoc.
//!
//! Pre-fix rubash let the assoc request win for scalar operands
//! (`declare -Ax ev=([0]="6" )`) and converted existing scalars to
//! arrays for name-only operands (`x=1; export -A x` -> `([0]="1")`).
//!
//! Expectations measured against GNU bash 5.3.20 (oracle binary),
//! script probes 2026-11-07.

use std::process::{Command, Stdio};
use std::time::Duration;

mod common;
use common::wait_bounded;

fn run(script: &str) -> (String, String, i32) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .env_clear()
        .env("LC_ALL", "C")
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let output = wait_bounded(&mut child, Duration::from_secs(20));
    assert!(!output.timed_out, "case timed out: {script}");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.code.unwrap_or(-1),
    )
}

fn declare_p(script: &str) -> String {
    let (stdout, stderr, status) = run(script);
    assert_eq!(status, 0, "script {script:?} failed: {stderr}");
    let _ = stderr;
    stdout.trim_end().to_string()
}

#[test]
fn export_aa_scalar_binds_indexed() {
    assert_eq!(
        declare_p("export -aA ev=6; declare -p ev"),
        "declare -ax ev=([0]=\"6\")"
    );
}

#[test]
fn export_aa_reversed_order_binds_indexed() {
    assert_eq!(
        declare_p("export -Aa ev=6; declare -p ev"),
        "declare -ax ev=([0]=\"6\")"
    );
}

#[test]
fn readonly_aa_scalar_binds_indexed() {
    assert_eq!(
        declare_p("readonly -aA rv=6; declare -p rv"),
        "declare -ar rv=([0]=\"6\")"
    );
}

#[test]
fn export_aa_scalar_word_binds_indexed() {
    assert_eq!(
        declare_p("export -aA sv=hello; declare -p sv"),
        "declare -ax sv=([0]=\"hello\")"
    );
}

#[test]
fn export_aa_over_existing_array_replaces_element_zero() {
    assert_eq!(
        declare_p("ev=(p q); export -aA ev=6; declare -p ev"),
        "declare -ax ev=([0]=\"6\" [1]=\"q\")"
    );
}

#[test]
fn export_aa_compound_stays_assoc() {
    // Control: compound operands were expanded under the assoc
    // interpretation; this already matched GNU before the fix.
    assert_eq!(
        declare_p("export -aA cv=(x y); declare -p cv"),
        "declare -Ax cv=([x]=\"y\" )"
    );
}

#[test]
fn export_assoc_only_scalar_binds_assoc() {
    // Control.
    assert_eq!(
        declare_p("export -A av=6; declare -p av"),
        "declare -Ax av=([0]=\"6\" )"
    );
}

#[test]
fn export_array_only_scalar_binds_indexed() {
    // Control.
    assert_eq!(
        declare_p("export -a av=6; declare -p av"),
        "declare -ax av=([0]=\"6\")"
    );
}

#[test]
fn export_assoc_name_only_keeps_scalar() {
    // No assignment operand: the -A flag is inert (setattr.def's
    // delegation runs only under `if (assign)`).
    assert_eq!(
        declare_p("x=1; export -A x; declare -p x"),
        "declare -x x=\"1\""
    );
}

#[test]
fn export_array_name_only_keeps_scalar() {
    assert_eq!(
        declare_p("x=1; export -a x; declare -p x"),
        "declare -x x=\"1\""
    );
}

#[test]
fn export_aa_name_only_fresh_is_plain_export() {
    // Control: fresh name-only operand just gains att_exported.
    assert_eq!(
        declare_p("export -aA fresh; declare -p fresh"),
        "declare -x fresh"
    );
}
