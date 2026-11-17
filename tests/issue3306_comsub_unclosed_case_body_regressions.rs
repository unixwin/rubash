//! rubash#3306 — command substitutions whose `case` body can never
//! complete: the `)` that GNU's parser takes as the substitution's
//! shell_eof_token (parse.y:4519-4521 parse_comsub) must close the
//! extraction even when the case bookkeeping would hold it — a clause
//! BODY or `case … in` preamble position (pattern region disarmed),
//! or a case already doomed by a pattern-position clause terminator
//! (`;;`/`;&`/`;;&` is only legal after a clause body). The truncated
//! body then reaches the body parser, whose verdict the read-time
//! screen reports exactly like GNU's nested yyparse
//! (parse.y:4549-4555, 6858-6865): a real token offender keeps
//! ` while looking for matching `)'`; an end-of-input shortfall names
//! the `)' itself.
//!
//! Before the fix the gate/close-char scanners scanned off the end of
//! the word and the reader reported a generic
//! `unexpected EOF while looking for matching `)'' — or, for the
//! `-c` string, swallowed the whole unit and reported at execution.
//!
//! Expectations measured against GNU bash 5.3.20 script-file probes
//! (2026-11, scratch matrix: 31/33 shapes byte-identical; the two
//! quoted-wrap residuals `"…$(case…)…"` are a separate adjacent
//! family — the dquote scanner never releases the inner `)` — and
//! are deliberately NOT pinned here).

use std::path::PathBuf;
use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_file(tag: &str, script: &str) -> Run {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash3306-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    std::fs::write(dir.join("case.sh"), script).expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("case.sh")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &dir)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

/// File mode: GNU exits 2, prints the diagnostic then the offending
/// line echo; nothing executes.
fn assert_file_diagnostic(tag: &str, run: &Run, expected_stderr: &str) {
    assert_eq!(run.stdout, "", "{tag}: stdout");
    assert_eq!(run.code, Some(2), "{tag}: rc");
    assert_eq!(run.stderr, expected_stderr, "{tag}: stderr");
}

#[test]
fn spaced_pattern_body_paren_is_eof_token() {
    // `a| b` is a complete pattern list; the `)` after `b` closes it,
    // `echo hi` is the clause body, and the final `)` is the
    // substitution's eof token — the body then dies on ITS end, so
    // the offender is `)' itself with no suffix.
    let run = run_file("a", "echo $(case x in a| b) echo hi)\n");
    assert_file_diagnostic(
        "a",
        &run,
        "case.sh: line 1: syntax error near unexpected token `)'\n\
         case.sh: line 1: `echo $(case x in a| b) echo hi)'\n",
    );
}

#[test]
fn nospace_pattern_body_paren_is_eof_token() {
    let run = run_file("b", "echo $(case x in a|b) echo hi)\n");
    assert_file_diagnostic(
        "b",
        &run,
        "case.sh: line 1: syntax error near unexpected token `)'\n\
         case.sh: line 1: `echo $(case x in a|b) echo hi)'\n",
    );
}

#[test]
fn pattern_terminator_in_pattern_position_keeps_suffix() {
    // `;;` while the pattern region is still armed is fatal in GNU's
    // grammar — a real token offender, so the `while looking for
    // matching `)'' suffix stays.
    for (tag, script) in [
        ("c", "echo $(case x in a;; echo hi)\n"),
        ("d", "echo $(case x in a;;)\n"),
        ("e", "echo $(case x in a ;;)\n"),
    ] {
        let run = run_file(tag, script);
        let line = script.trim_end();
        assert_file_diagnostic(
            tag,
            &run,
            &format!(
                "case.sh: line 1: syntax error near unexpected token `;;' while looking for matching `)'\n\
                 case.sh: line 1: `{line}'\n"
            ),
        );
    }
}

#[test]
fn doomed_case_then_later_esac_still_names_terminator() {
    // `a;;` dooms the case even though a later `esac` would complete
    // it — GNU dies on the `;;` token first.
    let run = run_file("f", "echo $(case x in a;; b) x;; esac)\n");
    assert_file_diagnostic(
        "f",
        &run,
        "case.sh: line 1: syntax error near unexpected token `;;' while looking for matching `)'\n\
         case.sh: line 1: `echo $(case x in a;; b) x;; esac)'\n",
    );
}

#[test]
fn empty_pattern_paren_names_close_paren() {
    // `()` is an empty pattern — the `)` IS the offending token (the
    // sub-parse's eof token), so no suffix.
    for (tag, script) in [
        ("g", "echo $(case x in ())\n"),
        ("h", "echo $(case x in ( ))\n"),
        ("i", "echo $(case x in () )\n"),
    ] {
        let run = run_file(tag, script);
        let line = script.trim_end();
        assert_file_diagnostic(
            tag,
            &run,
            &format!(
                "case.sh: line 1: syntax error near unexpected token `)'\n\
                 case.sh: line 1: `{line}'\n"
            ),
        );
    }
}

#[test]
fn body_parens_hold_the_close() {
    // `(b)` / `((1+2))` inside the clause body own their `)` first —
    // the substitution then runs out, which is the `)' eof report.
    for (tag, script) in [
        ("j", "echo $(case x in a) (b)\n"),
        ("k", "echo $(case x in a) ((1+2))\n"),
        ("l", "echo $(case x in a) $((1+2))\n"),
    ] {
        let run = run_file(tag, script);
        assert_eq!(run.stdout, "", "{tag}: stdout");
        assert_eq!(run.code, Some(2), "{tag}: rc");
        assert!(
            run.stderr
                .contains("unexpected EOF while looking for matching `)'"),
            "{tag}: {:?}",
            run.stderr
        );
    }
}

#[test]
fn missing_comsub_close_still_eof() {
    // No `)` at all: genuinely incomplete — the generic EOF report at
    // the EOF line, exactly like GNU.
    let run = run_file("m", "echo $(case x in a) echo x\n");
    assert_eq!(run.stdout, "", "m: stdout");
    assert_eq!(run.code, Some(2), "m: rc");
    assert_eq!(
        run.stderr,
        "case.sh: line 2: unexpected EOF while looking for matching `)'\n"
    );
}

#[test]
fn earlier_unit_command_does_not_run() {
    // GNU's reader validates the body while reading: the whole unit
    // dies before `echo ok' executes.
    let run = run_file("n", "echo ok; echo $(case x in a;; b)\n");
    assert_eq!(run.stdout, "", "n: stdout");
    assert_eq!(run.code, Some(2), "n: rc");
    assert!(
        run.stderr
            .contains("syntax error near unexpected token `;;' while looking for matching `)'"),
        "n: {:?}",
        run.stderr
    );
}

#[test]
fn complete_cases_stay_valid() {
    for (tag, script) in [
        ("o", "echo $(case x in a|b) x;; esac)\n"),
        ("p", "echo $(case x in a) echo x;; esac)\n"),
        ("q", "echo $(case x in (a|b) x;; esac)\n"),
        ("r", "echo $(case x in a) ( : );; esac)\n"),
    ] {
        let run = run_file(tag, script);
        // `echo` of an empty substitution still prints its newline.
        assert_eq!(run.stdout, "\n", "{tag}: stdout");
        assert_eq!(run.stderr, "", "{tag}: stderr");
        assert_eq!(run.code, Some(0), "{tag}: rc");
    }
}
