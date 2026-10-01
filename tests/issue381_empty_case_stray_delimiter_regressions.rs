//! rubash#381: empty-case strictness — `case x in` + newline + `esac)` was
//! silently accepted; GNU reports `syntax error near unexpected token `)'`.
//!
//! GNU spec: parse.y:1037 grammar `case WORD in newline_list CASE_ITEM...`
//! — the newline_list between `in` and the first pattern is legal, and
//! parse.y:3433-3441 (special_case_tokens) returns the `esac' after `in`
//! as the ESAC of the EMPTY case. The case command therefore ends at that
//! `esac', and whatever follows belongs to the OUTER grammar, which
//! rejects it: `)`/`(` directly after the finished case are `syntax error
//! near unexpected token X' at that token's line (yyerror names the
//! offending token; print_offending_line parse.y:6813-6826 echoes that
//! physical line), while `|` is a legal pipeline whose own tail errors at
//! its `)` (`case x in esac|y) ...` names the `)` after `y`). The same
//! rule applies between clauses: parse.y:3710 re-arms the pattern state at
//! `;;', so `case x in a) :;; esac) echo;;` is the same error.
//!
//! Both-directions coverage: the VALID neighbors must stay valid —
//! `case x in\n(esac) ...` (esac as a paren'd pattern) and the plain empty
//! case `case x in\nesac`.
//!
//! Expectations byte-verified against WSL GNU Bash 5.3.0 script-file
//! probes (target/probe381, 2026-10-02).

use std::process::Command;

fn rubash(script: &str) -> (String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stderr).replace("bash: -c: line ", "line "),
        output.status.code(),
    )
}

#[test]
fn newline_empty_case_then_stray_paren_errors() {
    // The #381 reproducer: previously accepted silently (rc 0, no output).
    let (stderr, code) = rubash("case x in\nesac) echo hi;;\nesac");
    assert_eq!(
        stderr,
        "line 2: syntax error near unexpected token `)'\nline 2: `esac) echo hi;;'\n"
    );
    assert_eq!(code, Some(2));
}

#[test]
fn one_line_form_still_errors_identically() {
    let (stderr, code) = rubash("case x in esac) echo hi;; esac");
    assert_eq!(
        stderr,
        "line 1: syntax error near unexpected token `)'\nline 1: `case x in esac) echo hi;; esac'\n"
    );
    assert_eq!(code, Some(2));
}

#[test]
fn multiple_newlines_and_stray_open_paren() {
    let (stderr, code) = rubash("case x in\n\nesac) echo hi;;\nesac");
    assert!(
        stderr.contains("syntax error near unexpected token `)'"),
        "{stderr}"
    );
    assert_eq!(code, Some(2));
    // `(` after the finished case names the `(`.
    let (stderr, code) = rubash("case x in\nesac(y) echo hi;;\nesac");
    assert!(
        stderr.contains("syntax error near unexpected token `('"),
        "{stderr}"
    );
    assert_eq!(code, Some(2));
}

#[test]
fn stray_pipe_errors_at_the_pipeline_tail_paren() {
    // `|` is a legal pipeline in the outer grammar: GNU's error surfaces
    // at the `)` after the tail command, and nothing executes.
    let (stderr, code) = rubash("case x in\nesac|y) echo hi;;\nesac");
    assert!(
        stderr.contains("syntax error near unexpected token `)'"),
        "{stderr}"
    );
    assert_eq!(code, Some(2));
}

#[test]
fn between_clauses_stray_paren_errors() {
    let (stderr, code) = rubash("case x in\na) echo A;;\nesac) echo hi;;\nesac");
    assert_eq!(
        stderr,
        "line 3: syntax error near unexpected token `)'\nline 3: `esac) echo hi;;'\n"
    );
    assert_eq!(code, Some(2));
}

#[test]
fn valid_neighbors_stay_valid() {
    // `esac` as a parenthesized pattern (reverse direction of the same
    // class): must keep parsing and matching.
    let (stderr, code) = rubash("case x in\n(esac) echo ok;;\nesac; echo done");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    // The plain empty case (with and without newlines) is a complete
    // command.
    for script in [
        "case x in\nesac\n echo done",
        "case x in esac; echo done",
        "case x in\n\nesac; echo done",
    ] {
        let (stderr, code) = rubash(script);
        assert_eq!(stderr, "", "{script}");
        assert_eq!(code, Some(0), "{script}");
    }
}

#[test]
fn while_body_reports_the_offending_line() {
    // The empty case inside a loop names the `esac)` line, not the loop
    // or the final `esac` line.
    let (stderr, code) = rubash("while :; do case x in\nesac) break;;\nesac; done; echo D");
    assert!(
        stderr.contains("line 2: syntax error near unexpected token `)'"),
        "{stderr}"
    );
    assert!(stderr.contains("line 2: `esac) break;;'"), "{stderr}");
    assert_eq!(code, Some(2));
}
