//! End-to-end regressions for the 2026-09-26 parse/expand/exec fixes on
//! wt/o1-carrier:
//! - rubash#187: mixed-quote word with the quoted span BEFORE the command
//!   substitution leaked its CTLESC \x11 marker into argv/output.
//! - rubash#188: a bad substitution inside `eval` abandoned the whole
//!   function body instead of only the eval'd string.
//! - rubash#181: `<<` inside an arithmetic command lexed as a here-document
//!   operator and swallowed the rest of the script.
//! - rubash#174: a newline inside a cross-line `(( ... ))` command folded to
//!   `;` and was fed to the arithmetic evaluator.

use std::process::Command;

fn rubash_raw(script: &str) -> Vec<u8> {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    output.stdout
}

fn rubash(script: &str) -> String {
    String::from_utf8_lossy(&rubash_raw(script)).into_owned()
}

// ===========================================================================
// rubash#187 — CTLESC carrier leak, mixed-quote word, quoted span first.
// GNU expand_word_internal carries quoted characters through the splice with
// a CTLESC prefix (subst.c:11639-11673); glob consumes the protection and
// dequote_string (subst.c:4807) drops it at argv materialization.
// ===========================================================================

#[test]
fn mixed_word_quoted_span_before_comsub_keeps_ctl_esc_out_of_output() {
    assert_eq!(rubash("echo pre\"a*\"$(echo hi)post"), "prea*hipost\n");
}

#[test]
fn mixed_word_quoted_span_before_comsub_has_no_0x11_byte() {
    let bytes = rubash_raw("printf %s pre\"a*\"$(echo hi)post");
    assert!(!bytes.contains(&0x11), "CTLESC leaked: {bytes:?}");
    assert_eq!(bytes, b"prea*hipost");
}

#[test]
fn mixed_word_quoted_span_before_comsub_still_suppresses_glob() {
    // With a file that would match the unquoted pattern, the quoted `*`
    // must keep the word literal (GNU: CTLESC-protected during pathname
    // expansion). Run in an empty temp dir so the file list is controlled.
    let dir = std::env::temp_dir().join(format!("rubash-o1-187-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join("preXhi"), b"").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("echo pre\"a*\"hi")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&output.stdout), "prea*hi\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn comsub_payload_0x11_is_data_not_marker() {
    // A literal 0x11 byte produced BY the substitution is payload data and
    // must still reach output byte-exact (owner-tagged through the splice).
    let bytes = rubash_raw("printf %s \"x$(printf '\\021')y\"");
    assert_eq!(bytes, b"x\x11y");
}

// ===========================================================================
// rubash#188 — eval bad substitution ends only the eval'd string.
// GNU evalstring.c:372 setjmp_nosigs(top_level); DISCARD arm 423-450 returns
// from parse_and_execute with EXECUTION_FAILURE outside a subshell.
// ===========================================================================

#[test]
fn eval_bad_substitution_does_not_abandon_function_body() {
    assert_eq!(
        rubash("f13() { eval 'x=${.sh.version}'; return 5; }\nf13 2>/dev/null\necho rc=$?"),
        "rc=5\n"
    );
}

#[test]
fn eval_bad_substitution_function_body_continues() {
    assert_eq!(
        rubash(
            "f2() { echo a; eval 'x=${.sh.version}'; echo b; return 7; }\nf2 2>/dev/null\necho rc=$?"
        ),
        "a\nb\nrc=7\n"
    );
}

#[test]
fn direct_bad_substitution_still_ends_function_invocation() {
    // The DIRECT form (no eval) ends the invocation: GNU's DISCARD unwinds
    // the function body to the caller; `return 5` never runs.
    assert_eq!(
        rubash("f3() { x=${.sh.version}; return 5; }\nf3 2>/dev/null\necho rc=$?"),
        "rc=1\n"
    );
}

// ===========================================================================
// rubash#181 — `<<` inside (( )) is the shift operator.
// GNU parse.y:2626 read_token routes `((` to parse_dparen (parse.y:3517+);
// the body is consumed by parse_matched_pair before the REDIR_LESSLESS
// branch (parse.y:2630+) can see the `<<`.
// ===========================================================================

#[test]
fn shift_assign_in_arithmetic_command_is_not_heredoc() {
    assert_eq!(rubash("x=8\n((x<<=2))\necho v=$x"), "v=32\n");
}

#[test]
fn shift_in_arithmetic_expansion() {
    assert_eq!(rubash("echo $(( 1<<4 ))"), "16\n");
    assert_eq!(rubash("echo $(( $(echo 4) << 1 ))"), "8\n");
}

#[test]
fn digit_shift_operand_in_arithmetic_command() {
    assert_eq!(rubash("(( 2<<3 == 16 )) && echo eq"), "eq\n");
    assert_eq!(
        rubash("if (( 2<<3 == 16 )); then echo shift-if; fi"),
        "shift-if\n"
    );
}

#[test]
fn heredoc_still_works_after_arithmetic_shift_line() {
    assert_eq!(rubash("(( 2<<3 == 16 ))\ncat <<Z\nbody\nZ"), "body\n");
}

#[test]
fn subshell_shaped_double_paren_keeps_heredoc() {
    // `((echo hi); cat <<X)` closes as a subshell, not an arithmetic
    // command: parse_dparen rejects it and the `<<` stays a here-document.
    assert_eq!(
        rubash("((echo hi); cat <<X)\nsubshell-heredoc\nX"),
        "hi\nsubshell-heredoc\n"
    );
}

// ===========================================================================
// rubash#174 — newline inside (( )) is whitespace, not a separator.
// expr.c's lexer treats '\n' as a blank; parse_dparen reads the body across
// physical lines.
// ===========================================================================

#[test]
fn cross_line_arithmetic_command_if_branch() {
    assert_eq!(
        rubash(
            "a=9 b=3 c=4 maj=5 min=3\nif (( a > maj\n\t\t|| (a == maj && c < min) ))\nthen echo lt-branch; else echo ge-branch; fi"
        ),
        "lt-branch\n"
    );
}

#[test]
fn cross_line_arithmetic_command_updates_variable() {
    assert_eq!(rubash("x=1\n(( x\n+\n1 ))\necho x=$x"), "x=1\n");
    assert_eq!(rubash("v=$(( 1\n || 0 ))\necho v=$v"), "v=1\n");
}

#[test]
fn literal_semicolon_in_arithmetic_command_still_errors() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("(( 1;2 ))\necho after=$?")
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&output.stdout), "after=1\n");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("arithmetic syntax error"),
        "expected arithmetic syntax error diagnostic, got: {stderr}"
    );
}
