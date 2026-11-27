//! Issue rubash#1965: a numbered-fd here-string (`0<<<`) stores its word
//! as already-decoded text whose `$'...'` blanks carry the U+E401
//! ANSI_C_IFS_GUARD carrier (`\u{1d}a<U+E401>\tb`). The two
//! stdin_string_for_command ladders in shell_options.rs decoded/expanded
//! that storage form WITHOUT the guard strip the canonical funnel applies
//! (expand_heredoc_redirect_text -> expand_stored_here_string_word), so on
//! the pipeline staging route the guard byte leaked end-to-end into the
//! delivered stream: `cat 0<<< $'a\tb' | od` showed an extra ee 90 81.
//!
//! GNU semantics: a here-string's word is written to the pipe as its
//! decoded data bytes plus a trailing newline (redir.c
//! here_document_to_fd) — no carrier byte of any kind is observable.
//! Expected outputs are byte-verified against GNU bash 5.3.20 under a
//! UTF-8 locale (LC_ALL=C.utf8, pinned by the harness).

use std::process::Command;

fn rubash_stdout(script: &str) -> Vec<u8> {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env("LC_ALL", "C.utf8")
        .output()
        .expect("run rubash");
    assert!(
        output.status.success(),
        "script failed: {script}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

/// The ticket reproduction: tab shape on the pipeline staging route.
#[test]
fn numbered_herestring_tab_pipeline_has_no_guard() {
    let out = rubash_stdout("cat 0<<< $'a\\tb' | od -An -tx1");
    assert_eq!(out, b" 61 09 62 0a\n".to_vec());
}

/// Space shape: the decoded blank is guarded in storage too.
#[test]
fn numbered_herestring_space_pipeline_has_no_guard() {
    let out = rubash_stdout("cat 0<<< $'a b' | od -An -tx1");
    assert_eq!(out, b" 61 20 62 0a\n".to_vec());
}

/// Newline shape: the decoded newline carries the guard in storage.
#[test]
fn numbered_herestring_newline_pipeline_has_no_guard() {
    let out = rubash_stdout("cat 0<<< $'a\\nb' | od -An -tx1");
    assert_eq!(out, b" 61 0a 62 0a\n".to_vec());
}

/// A USER-typed U+E401 is data, not the guard: it is E400-escaped in
/// storage, and the E400-aware strip must preserve it end-to-end.
#[test]
fn numbered_herestring_typed_e401_is_data() {
    let out = rubash_stdout("cat 0<<< $'a\\uE401b' | od -An -tx1");
    assert_eq!(out, " 61 ee 90 81 62 0a\n".to_string().into_bytes());
}

/// Control: the direct (non-pipeline) bind route was already clean and
/// must stay byte-identical.
#[test]
fn numbered_herestring_direct_route_stays_clean() {
    let out = rubash_stdout("cat 0<<< $'a\\tb'");
    assert_eq!(out, b"a\tb\n".to_vec());
}

/// Control: the unnumbered here-string pipeline form was already clean.
#[test]
fn unnumbered_herestring_pipeline_stays_clean() {
    let out = rubash_stdout("cat <<< $'a\\tb' | od -An -tx1");
    assert_eq!(out, b" 61 09 62 0a\n".to_vec());
}

/// Control: the comsub staging route was already clean (trailing newline
/// is consumed by the substitution itself, as in GNU).
#[test]
fn numbered_herestring_comsub_stays_clean() {
    let out = rubash_stdout("x=$(cat 0<<< $'a\\tb'); printf '%s' \"$x\" | od -An -tx1");
    assert_eq!(out, b" 61 09 62\n".to_vec());
}
