//! Issue rubash#1910: `shopt -s nocasematch` was ignored by the anchored
//! pattern-substitution forms `${v/#pat/rep}` and `${v/%pat/rep}` — the
//! value came back unchanged while GNU substitutes. The unanchored form
//! already honored the option.
//!
//! GNU semantics: pat_subst routes EVERY pattern through
//! match_pattern/match_upattern (subst.c:9238), and match_upattern
//! applies FNMATCH_IGNCASE whenever nocasematch is set (subst.c:5382) —
//! the anchored MATCH_BEG/MATCH_END scans get the same case folding as
//! the MATCH_ANY scan; there is no case-sensitive anchored variant.
//!
//! Root cause: `find_parameter_prefix_match` /
//! `find_parameter_suffix_match` in src/executor/parameter_replace.rs
//! hardcoded `false` for the matcher's nocase parameter, and the
//! `replace_parameter_prefix` / `replace_parameter_suffix` wrappers did
//! not take the flag at all, while the unanchored finder threaded it
//! through. Fix: thread the caller's `nocase` through both anchored
//! paths, exactly like the unanchored one.
//!
//! Expected outputs are byte-verified against GNU Bash 5.3.20
//! (self-compiled oracle, script-file probes 2026-10-28).

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

/// The issue's exact seed: anchored prefix under nocasematch.
#[test]
fn anchored_prefix_honors_nocasematch() {
    let (stdout, stderr, _) = rubash("shopt -s nocasematch\nv=seg/seg/x\necho \"${v/#SEG/Q}\"");
    assert_clean(&stderr);
    assert_eq!(stdout, "Q/seg/x\n");
}

/// The issue's suffix twin.
#[test]
fn anchored_suffix_honors_nocasematch() {
    let (stdout, stderr, _) = rubash("shopt -s nocasematch\nv=seg/seg/x\necho \"${v/%X/Y}\"");
    assert_clean(&stderr);
    assert_eq!(stdout, "seg/seg/Y\n");
}

/// Case folding applies to the VALUE's case as well, both anchors.
#[test]
fn anchored_forms_fold_value_case() {
    let (stdout, stderr, _) = rubash(
        "shopt -s nocasematch\nv=HELLO/world\necho \"${v/#hello/Q}\"\necho \"${v/%WORLD/Z}\"",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "Q/world\nHELLO/Z\n");
}

/// Glob patterns in anchored position fold case too.
#[test]
fn anchored_glob_pattern_honors_nocasematch() {
    let (stdout, stderr, _) =
        rubash("shopt -s nocasematch\nv=seg/seg/x\necho \"${v/#S?G/Q}\"\necho \"${v/%[x]/Y}\"");
    assert_clean(&stderr);
    assert_eq!(stdout, "Q/seg/x\nseg/seg/Y\n");
}

/// With nocasematch off the anchored forms stay case-sensitive.
#[test]
fn anchored_forms_case_sensitive_without_nocasematch() {
    let (stdout, stderr, _) = rubash("v=seg/seg/x\necho \"${v/#SEG/Q}\"\necho \"${v/%X/Y}\"");
    assert_clean(&stderr);
    assert_eq!(stdout, "seg/seg/x\nseg/seg/x\n");
}

/// The unanchored form keeps working under nocasematch (was already
/// correct; pin it so the anchored fix cannot regress it).
#[test]
fn unanchored_form_still_honors_nocasematch() {
    let (stdout, stderr, _) = rubash("shopt -s nocasematch\nv=seg/seg/x\necho \"${v/SEG/Q}\"");
    assert_clean(&stderr);
    assert_eq!(stdout, "Q/seg/x\n");
}
