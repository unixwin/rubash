//! Issue rubash#517 — xtrace case-head word fidelity.
//!
//! GNU `xtrace_print_case_command_head` (print_cmd.c:742-749) prints
//! `case_command->word->word` — the WORD_DESC text, which in GNU still
//! carries the source quote characters, so `case "$x" in` traces as
//! `+ case "$x" in`. Rubash traced the lexer's quote-stripped `.word`
//! value instead, whose PARAM_NAME_END_MARKER boundary byte (0x13, where
//! a deleted closing quote terminated an unbraced `$name`) leaked raw
//! into the trace: `+ case $x^S in`.
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (script-file probes, 2026-10-12).

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

/// A quoted case head keeps its quotes in the trace (GNU `+ case "$x" in`).
#[test]
fn xtrace_case_head_keeps_quotes() {
    let (_stdout, stderr, code) = rubash("set -x; x=hi; case \"$x\" in hi) :;; esac");
    assert_eq!(stderr, "+ x=hi\n+ case \"$x\" in\n+ :\n");
    assert_eq!(code, Some(0));
}

/// An unquoted case head stays unquoted (GNU `+ case $x in`).
#[test]
fn xtrace_case_head_unquoted_stays_plain() {
    let (_stdout, stderr, code) = rubash("set -x; x=hi; case $x in hi) :;; esac");
    assert_eq!(stderr, "+ x=hi\n+ case $x in\n+ :\n");
    assert_eq!(code, Some(0));
}

/// No internal marker byte may reach the trace output (the 0x13 leak).
#[test]
fn xtrace_case_head_carries_no_marker_bytes() {
    let (_stdout, stderr, _code) = rubash("set -x; y=\"a b\"; case \"$y\" in *) :;; esac");
    assert!(
        !stderr
            .bytes()
            .any(|byte| (0x00..=0x1f).contains(&byte) && byte != b'\n'),
        "control byte leaked into xtrace: {stderr:?}"
    );
    assert!(stderr.contains("+ case \"$y\" in\n"), "stderr={stderr:?}");
}
