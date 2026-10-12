//! Issue #512: `$((` must fall back to command substitution when the
//! parenthesis group opened by the second `(` does not close immediately
//! before a `)`.
//!
//! GNU spec (parse.y:4976 parse_arith_cmd via parse_matched_pair): after
//! `$((`, bash reads the balanced group and inspects the character right
//! after the close. Only `)` makes the construct arithmetic
//! (`$((1+1))`); anything else re-lexes the whole construct as a command
//! substitution whose body starts at the second `(` — `$((echo hi) 2>&1)`
//! runs `(echo hi) 2>&1`, prints `hi`, status 0. Corpus F158/F159
//! (dollar_paren_subshell_diff_check.sh:17-18) pinned `hi`/`A` with rc 0;
//! rubash used to force the arithmetic reading and die with
//! `arithmetic syntax error` rc 1. The verdict also bounds the WORD scan:
//! `echo $((echo hi) 2>&1); echo done` must run the trailing `echo done`,
//! which the old always-arithmetic swallow consumed into one word.
//!
//! WSL GNU Bash 5.3.0 measured oracle for every case below (2026-10-12):
//! `hi`/`A`/`hix`/`[hi]` stdout, `2`/`3` for the arithmetic controls, and
//! `$(((echo hi)))` stays arithmetic (`(echo hi): missing ')'`
//! diagnostic, rc 0 — the verdict char after its group IS `)`).
//!
//! Rust semantic owner: lexer/skip.rs dollar_double_paren_is_arithmetic
//! (verdict), lexer/scanner.rs `$(` branch (word span),
//! executor/parameter_ops.rs whole_word_arithmetic_substitution_body
//! (whole-word admission decline), embedded walker force_comsub arm
//! (execution of the declined form).

#![cfg(windows)]

use std::io::Write;
use std::process::{Command, Stdio};

struct RunOutcome {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_rubash_script(script: &str) -> RunOutcome {
    let dir = std::env::temp_dir().join(format!("rubash-issue512-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue512 scratch dir");
    let script_path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&script_path).expect("create probe script");
    file.write_all(script.as_bytes())
        .expect("write probe script");
    drop(file);
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash script");
    let code = output.status.code();
    let _ = std::fs::remove_dir_all(&dir);
    RunOutcome {
        code,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Corpus F158: the headline fallback shape, with the trailing `; echo`
/// proving the substitution ends at its own `)` (the old swallow consumed
/// the rest of the line into the word).
#[test]
fn issue512_redirect_fallback_runs_command_substitution() {
    let out = run_rubash_script("echo $((echo hi) 2>&1); echo \"rc=$?\"\n");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "hi\nrc=0\n");
    assert_eq!(out.stderr, "");
}

/// Corpus F159: fallback body participates in a pipeline.
#[test]
fn issue512_fallback_body_in_pipeline() {
    let out = run_rubash_script("echo $((echo a) | tr a-z A-Z)\n");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "A\n");
    assert_eq!(out.stderr, "");
}

/// The quoted form falls back the same way (GNU prints `hi` from the
/// quoted whole-word fast path).
#[test]
fn issue512_quoted_form_falls_back() {
    let out = run_rubash_script("echo \"$((echo hi) 2>&1)\"\n");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "hi\n");
    assert_eq!(out.stderr, "");
}

/// Word text after the substitution close stays outside it (GNU `hix`).
#[test]
fn issue512_trailing_word_text_stays_literal() {
    let out = run_rubash_script("echo $((echo hi) 2>&1)x\n");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "hix\n");
    assert_eq!(out.stderr, "");
}

/// Assignment RHS takes the fallback too (GNU stores `hi`).
#[test]
fn issue512_assignment_rhs_falls_back() {
    let out = run_rubash_script("x=$((echo hi) 2>&1)\necho \"[$x]\"\n");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "[hi]\n");
    assert_eq!(out.stderr, "");
}

/// Arithmetic controls keep the arithmetic reading: verdict char after the
/// group is `)`, so these evaluate (GNU 2 / 3).
#[test]
fn issue512_arithmetic_forms_still_evaluate() {
    let out = run_rubash_script("echo $((1+1))\necho $(( (1+2) ))\n");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "2\n3\n");
    assert_eq!(out.stderr, "");
}

/// `$(((echo hi)))`: the group `(echo hi)` closes right before `)` — the
/// verdict keeps it ARITHMETIC (GNU: `(echo hi): missing ')'` diagnostic
/// with error token `hi)`, rc 0, script continues).
#[test]
fn issue512_triple_paren_stays_arithmetic() {
    let out = run_rubash_script("echo $(((echo hi)))\necho after=$?\n");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "after=1\n");
    assert!(
        out.stderr.contains("missing `)'"),
        "expected GNU missing-paren arithmetic diagnostic, got {:?}",
        out.stderr
    );
}
