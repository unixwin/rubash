//! niubash defect D11 (stage-log §235) regression: a pipeline member's
//! argument word that mixes a backtick command substitution with an
//! apostrophe reached the child's argv with the DATA_SQUOTE carrier
//! (\x17, U+0017) still encoded — `member^Ws` instead of `member's`.
//! Git Bash prints `member's`; rubash printed `'A AAA B member\x17s C'`.
//!
//! Mechanism: the lexer encodes a `'` inside a double-quoted word that
//! also carries a live backtick substitution as DATA_SQUOTE
//! (lexer/quotes.rs:208). The plain-command argv boundary materializes
//! it back via `materialize_expanded_command_word` (command_prepare.rs),
//! but the pipeline spawn faces — the sequential external stage
//! (pipeline_stages.rs) and the concurrent fast path
//! (pipeline_exec.rs concurrent_external_stage_spec) — only ran
//! `restore_command_substitution_output`, whose replace chain covers
//! DATA_BACKTICK / DATA_DOLLAR / PROTECTED_BACKSLASH / DATA_BACKSLASH /
//! DATA_DQUOTE and never DATA_SQUOTE.
//!
//! Trigger elements (stage-log §235): (1) an apostrophe inside the
//! double-quoted word, (2) a live backtick substitution in the same
//! word (`$()` does not trigger), (3) the command runs as a pipeline
//! member. The observer is a real external python child printing
//! argv[1] with repr(), so a raw \x17 byte cannot hide; probes skip
//! when the interpreter is absent (issue2793 convention).

#![cfg(windows)]

use std::process::{Command, Stdio};

#[derive(Debug)]
struct RunOutcome {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn rubash_c(script: &str) -> RunOutcome {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash -c");
    RunOutcome {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

/// The D11 probe: `payload` rides as one double-quoted word (the
/// apostrophe therefore sits inside the double-quoted word) handed to a
/// real external python child as argv[1]; the child repr()s it so a
/// leaked carrier byte shows up as `\x17`. `sink` is the pipeline tail.
fn d11_probe(payload: &str, sink: &str) -> RunOutcome {
    let line = format!(
        "python -c \"import sys; print(ascii(sys.argv[1]))\" \"{payload}\" | {sink}"
    );
    rubash_c(&line)
}

fn python_available() -> bool {
    Command::new("python")
        .arg("-c")
        .arg("pass")
        .stdin(Stdio::null())
        .output()
        .is_ok()
}

#[test]
fn d11_pipeline_backtick_squote_argv_is_clean() {
    if !python_available() {
        return;
    }
    // stage-log §235 minimal repro: the full three-element trigger
    // (apostrophe + backtick substitution + pipeline), sequential
    // stage face with `tail` as the member.
    let outcome = d11_probe("A `echo AAA` B member's", "tail -1");
    assert_eq!(outcome.code, Some(0), "{outcome:?}");
    assert_eq!(outcome.stdout.trim(), "\"A AAA B member's\"");
}

#[test]
fn d11_concurrent_face_backtick_squote_argv_is_clean() {
    if !python_available() {
        return;
    }
    // External-only tail: the member spawns on the concurrent fast
    // path (pipeline_exec.rs concurrent_external_stage_spec), the
    // second spawn face named in the mechanism.
    let outcome = d11_probe("A `echo AAA` B member's", "cat");
    assert_eq!(outcome.code, Some(0), "{outcome:?}");
    assert_eq!(outcome.stdout.trim(), "\"A AAA B member's\"");
}

#[test]
fn d11_pipeline_multiple_squotes_all_restored() {
    if !python_available() {
        return;
    }
    // stage-log §235 multi-apostrophe shape: every carrier restored.
    let outcome = d11_probe("it's a `echo Z` test's", "tail -1");
    assert_eq!(outcome.code, Some(0), "{outcome:?}");
    assert_eq!(outcome.stdout.trim(), "\"it's a Z test's\"");
}

#[test]
fn d11_pipeline_squote_without_backtick_stays_clean() {
    if !python_available() {
        return;
    }
    // Element (1) alone — no live substitution in the word. The lexer
    // then leaves the apostrophe literal and the child must still see
    // it verbatim; guards a fix that decodes too eagerly.
    let outcome = d11_probe("member's", "tail -1");
    assert_eq!(outcome.code, Some(0), "{outcome:?}");
    assert_eq!(outcome.stdout.trim(), "\"member's\"");
}

#[test]
fn d11_pipeline_dollar_paren_squote_stays_clean() {
    if !python_available() {
        return;
    }
    // Element (2) variant: `$()` never leaked (the substitution walker
    // restores on that shape); keep the good behavior pinned.
    let outcome = d11_probe("A $(echo AAA) B member's", "tail -1");
    assert_eq!(outcome.code, Some(0), "{outcome:?}");
    assert_eq!(outcome.stdout.trim(), "\"A AAA B member's\"");
}

#[test]
fn d11_no_pipe_backtick_squote_argv_stays_clean() {
    if !python_available() {
        return;
    }
    // Elements (1)+(2) without the pipeline: the plain-command argv
    // boundary already decoded (command_prepare.rs
    // materialize_expanded_command_word); unchanged by the fix.
    let outcome = rubash_c(
        "python -c \"import sys; print(ascii(sys.argv[1]))\" \"A `echo AAA` B member's\"",
    );
    assert_eq!(outcome.code, Some(0), "{outcome:?}");
    assert_eq!(outcome.stdout.trim(), "\"A AAA B member's\"");
}
