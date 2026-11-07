//! Deep-audit lane A1 (parser & grammar edges) — regression pins for the
//! four findings fixed on branch audit/a1-parser. Every expectation was
//! measured against GNU bash 5.3.20 (the project oracle); see
//! ~/workspace/deep-audit/a1/REPORT.md for the full evidence.
//!
//! UNVERIFIED-BY-CAPTAIN.

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// F03: only a newline_list may stand between the case subject and `in`;
/// a stray `esac` there is a syntax error, not a skippable token.
#[test]
fn audit_a1_f03_case_word_esac_in_is_syntax_error() {
    let (stdout, stderr, code) = rubash("case a esac in a) echo WEIRD;; esac; echo rc=$?");
    assert_eq!(code, Some(2), "stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(stdout, "");
    assert!(stderr.contains("syntax error"), "{stderr:?}");
}

/// F03: a second subject word is likewise rejected (GNU names it).
#[test]
fn audit_a1_f03_case_two_subject_words_is_syntax_error() {
    let (stdout, _stderr, code) = rubash("case a b in b) echo X;; esac");
    assert_eq!(code, Some(2), "stdout={stdout:?}");
    assert_eq!(stdout, "");
}

/// F03 guard-rail: the legal shapes must keep working — quoted subject
/// with a space, newline before `in`, command-substitution subject.
#[test]
fn audit_a1_f03_case_legal_headers_still_parse() {
    let (stdout, _, code) = rubash("case \"a b\" in \"a b\") echo QUOTED;; esac");
    assert_eq!((stdout.as_str(), code), ("QUOTED\n", Some(0)));
    let (stdout, _, code) = rubash("case x\nin x) echo NL;; esac");
    assert_eq!((stdout.as_str(), code), ("NL\n", Some(0)));
    let (stdout, _, code) = rubash("case $(echo a) in a) echo SUB;; esac");
    assert_eq!((stdout.as_str(), code), ("SUB\n", Some(0)));
}

/// F06: a function definition is never a coproc body — unnamed, named,
/// and subshell-body spellings are all syntax errors in GNU.
#[test]
fn audit_a1_f06_coproc_function_definition_is_syntax_error() {
    for script in [
        "coproc f() { echo c; }; echo rc=$?",
        "coproc CP f() { echo x; }; echo rc=$?",
        "coproc f() ( echo x ); echo rc=$?",
        "coproc ( ); echo rc=$?",
    ] {
        let (stdout, stderr, code) = rubash(script);
        assert_eq!(
            code,
            Some(2),
            "{script}: stdout={stdout:?} stderr={stderr:?}"
        );
        assert_eq!(stdout, "", "{script}");
        assert!(stderr.contains("syntax error"), "{script}: {stderr:?}");
    }
}

/// F06 guard-rail: the legitimate coproc forms still work.
#[test]
fn audit_a1_f06_coproc_legal_forms_still_work() {
    let (stdout, _, code) =
        rubash("coproc cat; echo x >&${COPROC[1]}; read -u ${COPROC[0]} l; echo \"cp:$l\"");
    assert_eq!((stdout.as_str(), code), ("cp:x\n", Some(0)));
    let (stdout, _, code) = rubash(
        "coproc MYCP { cat; }; echo hi >&${MYCP[1]}; read -u ${MYCP[0]} l; echo \"named:$l\"",
    );
    assert_eq!((stdout.as_str(), code), ("named:hi\n", Some(0)));
    let (stdout, _, code) =
        rubash("coproc ( echo subcop ); read -u ${COPROC[0]} l; echo \"sub:$l\"");
    assert_eq!((stdout.as_str(), code), ("sub:subcop\n", Some(0)));
}

/// F13: an escaped blank inside the delimiter word is delimiter text —
/// the terminator line must match and the body must not run to EOF.
#[test]
fn audit_a1_f13_heredoc_delimiter_with_escaped_space_terminates() {
    let (stdout, stderr, code) = rubash("cat <<E\\ OF\nbody\nE OF\n");
    assert_eq!(
        (stdout.as_str(), code),
        ("body\n", Some(0)),
        "stderr={stderr:?}"
    );
    assert_eq!(stderr, "", "{stderr:?}");
    let (stdout, _, code) = rubash("cat <<\\ EOF\nbody\n EOF\n");
    assert_eq!((stdout.as_str(), code), ("body\n", Some(0)));
}

/// F14: an arithmetic-for body is a compound_list and cannot be empty,
/// exactly like while/until/for-in.
#[test]
fn audit_a1_f14_arithmetic_for_empty_body_is_syntax_error() {
    let (stdout, _stderr, code) = rubash("for ((i=0;i<1;i++)); do done; echo rc=$?");
    assert_eq!(code, Some(2), "stdout={stdout:?}");
    assert_eq!(stdout, "");
    // and the non-empty form still runs
    let (stdout, _, code) = rubash("for ((i=0;i<3;i++)); do echo $i; done");
    assert_eq!((stdout.as_str(), code), ("0\n1\n2\n", Some(0)));
}
