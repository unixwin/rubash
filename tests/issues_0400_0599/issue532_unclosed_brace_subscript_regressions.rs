//! Issue rubash#532 — a malformed `${x[}` is a RUNTIME bad substitution,
//! not a parse-time whole-script abort.
//!
//! GNU's word level matches the `${`...`}` pair only (parse.y:3877
//! read_token_word -> parse_matched_pair ('{', '}')); a `[` inside the body
//! is parameter text and never opens the P_ARRAYSUB subscript hunt
//! (parse.y:5635 runs for a bare-word subscript). The malformed reference
//! therefore reaches the expansion, which reports `${x[}: bad
//! substitution` with status 1 and abandons only the current command
//! list: `echo BEFORE; echo ${x[}; echo AFTER` runs BEFORE (the rest of
//! that same-line list dies, rc 1), while on separate lines AFTER still
//! runs (rc 0). rubash's subscript walk used to open a subscript hunt at
//! the body's `[` (the `{` reset the command position mid-word) and die at
//! parse time with `unexpected EOF while looking for matching `]'` rc 2 —
//! nothing in the script ran.
//!
//! The lexer-side fix mirrors the skip.rs oracle arm into the
//! continuation.rs incremental state machine (both places consume the
//! `${`...`}` body as one unit); the runtime arm extends the rubash#536
//! reference validation: an unclosed subscript bracket is a bad
//! substitution.
//!
//! Every case is byte-verified against WSL GNU Bash 5.3.0 (script-file
//! oracle, 2026-10-12). The backtick half of the parse/runtime family and
//! the stdin-rc family are separate issues (#515, #508) and stay out of
//! scope.

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let path = std::env::temp_dir().join(format!("rubash532_{}.sh", std::process::id()));
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// #532 repro: the same-line list runs the commands BEFORE the malformed
/// expansion, reports the runtime bad substitution, and stops the line
/// with rc 1 (GNU: `BEFORE`, rc 1, no AFTER).
#[test]
fn same_line_list_keeps_prefix_side_effects() {
    let (stdout, stderr, code) = rubash("echo BEFORE; echo ${x[}; echo AFTER");
    assert_eq!(stdout, "BEFORE\n");
    assert!(stderr.contains("${x[}: bad substitution"), "{stderr:?}");
    assert!(!stderr.contains("unexpected EOF"), "{stderr:?}");
    assert_eq!(code, Some(1));
}

/// #532: on separate lines the malformed command dies alone and the
/// script continues with rc 0 (GNU: BEFORE, the diagnostic at line 2,
/// AFTER, rc 0).
#[test]
fn separate_lines_continue_after_the_error() {
    let (stdout, stderr, code) = rubash("echo BEFORE\necho ${x[}\necho AFTER");
    assert_eq!(stdout, "BEFORE\nAFTER\n");
    assert!(
        stderr.contains("line 2: ${x[}: bad substitution"),
        "{stderr:?}"
    );
    assert_eq!(code, Some(0));
}

/// #532: inside a loop the error abandons the whole loop (the remaining
/// iterations die with it) but the next top-level command runs (GNU:
/// `iter 1`, then AFTERLOOP, rc 0).
#[test]
fn loop_abandons_but_script_continues() {
    let (stdout, stderr, code) =
        rubash("for i in 1 2; do echo \"iter $i\"; echo ${x[}; done\necho AFTERLOOP");
    assert_eq!(stdout, "iter 1\nAFTERLOOP\n");
    assert!(stderr.contains("${x[}: bad substitution"), "{stderr:?}");
    assert_eq!(code, Some(0));
}

/// #532: the unclosed-bracket class covers the indirect, length, and
/// arithmetic-body prefixes (`${!x[}`, `${#x[}`, `${a[$((1/0))}`).
#[test]
fn unclosed_bracket_prefix_variants_report() {
    for script in [
        "echo BEFORE\necho ${!x[}\necho AFTER",
        "echo BEFORE\necho ${#x[}\necho AFTER",
        "echo BEFORE\necho ${a[$((1/0))}\necho AFTER",
    ] {
        let (stdout, stderr, code) = rubash(script);
        assert_eq!(stdout, "BEFORE\nAFTER\n", "{script:?}");
        assert!(stderr.contains("bad substitution"), "{script:?} {stderr:?}");
        assert!(!stderr.contains("unexpected EOF"), "{script:?} {stderr:?}");
        assert_eq!(code, Some(0), "{script:?}");
    }
}

/// #532 boundary kept: valid subscripts, literal unclosed brackets in
/// argument words, and unclosed `${` (the `}` EOF class) behave exactly as
/// before.
#[test]
fn neighboring_lexer_classes_unchanged() {
    let (stdout, stderr, code) = rubash("x=(1 2); echo ${x[0]} ${x[@]} ${#x[@]}\necho ok");
    assert_eq!(stdout, "1 1 2 2\nok\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    let (stdout, stderr, _) = rubash("echo a[b literal\necho ok");
    assert_eq!(stdout, "a[b literal\nok\n");
    assert_eq!(stderr, "");
    let (_, stderr, code) = rubash("echo ${x");
    assert!(
        stderr.contains("unexpected EOF while looking for matching `}'"),
        "{stderr:?}"
    );
    assert_eq!(code, Some(2));
}
