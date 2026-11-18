//! Regression pins for rubash#3084 — a double-quoted case subject
//! loses a backslash that GNU keeps: in
//!   unset v; case "${v=a\ b}" in "a b") ... ;;
//! the subject in GNU is `a\ b` (the assignment binds that value
//! too), so the `"a b"` pattern does NOT match; rubash's case-word
//! pipeline drops the backslash, the subject becomes `a b`, and
//! the wrong clause runs.
//!
//! GNU ownership: the case word is expanded with
//! expand_word_leave_quoted and dequote_string
//! (execute_cmd.c:3695-3699). Inside double quotes a backslash is
//! an escape ONLY before `$`, backtick, `"`, `\` or newline
//! (parse.y read_token_word's double-quote scanner); before any
//! other character — a space included — it is literal data, and
//! that holds inside the word of `${v=...}` as well
//! (subst.c:7663 parameter_brace_expand_word). The same expansion
//! outside a case command (`echo "${v=a\ b}"`) already agrees with
//! GNU in rubash; only the case-word path — quote_aware_case_word
//! (src/executor/compound_exec.rs) feeding expand_case_word —
//! diverges, which is what these cells pin.
//!
//! All expectations are the literal GNU bash 5.3.20 outputs for
//! the same script files.
#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3084-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn run_script(tag: &str, body: &str) -> (String, String, Option<i32>) {
    let dir = scratch(tag);
    let script = dir.join("probe.sh");
    std::fs::write(&script, body).expect("write script");
    let output = Command::new(BIN)
        .arg(&script)
        .current_dir(&dir)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    let result = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn issue3084_dq_assignment_subject_keeps_backslash() {
    // The issue's reproducer: subject `a\ b` does not match the
    // `"a b"` pattern, and the bound value keeps the backslash.
    let (stdout, stderr, code) = run_script(
        "repro",
        "unset v; case \"${v=a\\ b}\" in \"a b\") echo \"match v=[$v]\";; *) echo \"nomatch v=[$v]\";; esac\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "nomatch v=[a\\ b]\n");
}

#[test]
fn issue3084_dq_subject_matches_backslash_pattern() {
    // The quoted pattern 'a\ b' is a literal backslash: the kept
    // backslash in the subject matches it.
    let (stdout, stderr, code) = run_script(
        "backslash-pattern",
        "unset v; case \"${v=a\\ b}\" in 'a\\ b') echo \"match v=[$v]\";; *) echo \"nomatch v=[$v]\";; esac\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "match v=[a\\ b]\n");
}

#[test]
fn issue3084_unquoted_subject_still_escapes() {
    // Control: unquoted, `\ ` IS an escape — the word is `a b`
    // and the `"a b"` pattern matches. This arm must not change.
    let (stdout, stderr, code) = run_script(
        "unquoted",
        "unset v; case ${v=a\\ b} in \"a b\") echo \"match v=[$v]\";; *) echo \"nomatch v=[$v]\";; esac\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "match v=[a b]\n");
}

#[test]
fn issue3084_plain_dq_literal_subject() {
    // No expansion at all: a plain double-quoted literal subject
    // keeps its backslash before a space too.
    let (stdout, stderr, code) = run_script(
        "literal",
        "case \"a\\ b\" in \"a b\") echo match;; *) echo nomatch;; esac\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "nomatch\n");
}

#[test]
fn issue3084_dq_legit_escapes_still_resolve() {
    // Over-correction guards: the escapes that ARE special inside
    // double quotes must keep resolving — `\$` is a literal `$`,
    // and `\\` collapses to one backslash.
    let (stdout, stderr, code) = run_script(
        "dollar-escape",
        "case \"a\\$b\" in 'a$b') echo match;; *) echo nomatch;; esac\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "match\n");

    let (stdout, stderr, code) = run_script(
        "backslash-escape",
        "case \"a\\\\b\" in 'a\\b') echo match;; *) echo nomatch;; esac\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "match\n");
}

#[test]
fn issue3084_non_case_control_and_chain() {
    // Control: the same expansion outside a case command already
    // keeps the backslash (the ordinary double-quote pipeline).
    let (stdout, stderr, code) =
        run_script("control", "unset v; echo \"${v=a\\ b}\"; echo \"v=[$v]\"\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "a\\ b\nv=[a\\ b]\n");

    // Chain: the value bound while expanding the case subject is
    // the value later commands read back — backslash included.
    let (stdout, stderr, code) = run_script(
        "chain",
        "unset v; case \"${v=a\\ b}\" in *) echo \"inv=[$v]\";; esac; echo \"after=[$v]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "inv=[a\\ b]\nafter=[a\\ b]\n");
}
