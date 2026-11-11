//! rubash#3229 — an unclosed extglob group at parse time reports the
//! wrong diagnostic: GNU reports the matched-pair EOF error
//! `unexpected EOF while looking for matching `)'` at the line the
//! group opened; rubash reported `syntax error near unexpected token
//! `('` (the `(`-token error of the extglob-OFF shape).
//!
//! GNU anchors: parse.y:5466-5489 `read_token_word` — with
//! `extended_glob` live and a PATTERN_CHAR (`@ * + ? !`, syntax.h:90-92)
//! before `(`, the group body is consumed by `parse_matched_pair(cd,
//! '(', ')', ...)`; on EOF that scan reports `unexpected EOF while
//! looking for matching `%c'` with close=')' (parse.y:3901-3912) at the
//! line the pair scan started, and read_token_word bails immediately
//! (return -1). rc is 2, nothing on the failing line or later runs,
//! complete commands before it have already executed.
//!
//! Harness: script files (the parse-time extglob gate opens only when
//! the `shopt -s extglob` line has executed before the group line is
//! read; a `-c` one-liner parses before the builtin runs). Same
//! `rubash_file` shape as issue3050. Expectations verified byte-for-byte
//! against GNU bash 5.3.20 script-file probes (census in the lane
//! report: five operators x {command, assignment, conditional, case}
//! positions x {plain, bracket} bodies, plus controls).

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue3229-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

const OPS: [&str; 5] = ["@", "*", "+", "?", "!"];

fn check_eof(script: &str, line: usize) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(2), "script: {script}\nstderr: {stderr}");
    assert_eq!(stdout, "", "script: {script}");
    assert_eq!(
        stderr,
        format!("case.sh: line {line}: unexpected EOF while looking for matching `)'\n"),
        "script: {script}"
    );
}

/// The issue's regression seed, verbatim shape: command position,
/// `@(ab` with no bracket anywhere.
#[test]
fn unclosed_group_command_position() {
    for op in OPS {
        check_eof(&format!("shopt -s extglob\necho {op}(ab\necho alive\n"), 2);
    }
}

/// Same with an unclosed `[` inside the group body (the #3208 g8
/// shape): the bracket is group text, the EOF still belongs to `)`.
#[test]
fn unclosed_group_command_position_with_bracket() {
    for op in OPS {
        check_eof(&format!("shopt -s extglob\necho {op}(a[b\necho alive\n"), 2);
    }
}

/// Assignment position: `x=@(ab` — read_token_word reads the value
/// word the same way, so the same matched-pair EOF owns the report.
#[test]
fn unclosed_group_assignment_position() {
    for op in OPS {
        check_eof(&format!("shopt -s extglob\nx={op}(ab\necho alive\n"), 2);
        check_eof(&format!("shopt -s extglob\nx={op}(a[b\necho alive\n"), 2);
    }
}

/// Conditional position: `[[ ab == @(ab ]]` — the group scan runs to
/// EOF (`]]` does not close a pattern group); GNU's conditional
/// parser then also reports its dangling-operator error at the EOF
/// line (one past the last script line).
#[test]
fn unclosed_group_conditional_position() {
    for op in OPS {
        let script = format!("shopt -s extglob\n[[ ab == {op}(ab ]]\necho alive\n");
        let (stdout, stderr, code) = rubash_file(&script);
        assert_eq!(code, Some(2), "script: {script}\nstderr: {stderr}");
        assert_eq!(stdout, "", "script: {script}");
        assert_eq!(
            stderr,
            "case.sh: line 2: unexpected EOF while looking for matching `)'\n\
             case.sh: line 4: unexpected argument to conditional binary operator\n",
            "script: {script}"
        );
    }
}

/// Case-pattern position: the pattern group opens on line 3 and never
/// closes; the report is at the group's line.
#[test]
fn unclosed_group_case_position() {
    for op in OPS {
        check_eof(
            &format!("shopt -s extglob\ncase ab in\n{op}(ab\necho HIT;;\nesac\necho alive\n"),
            3,
        );
        check_eof(
            &format!("shopt -s extglob\ncase ab in\n{op}(a[b\necho HIT;;\nesac\necho alive\n"),
            3,
        );
    }
}

/// The group as the command word itself.
#[test]
fn unclosed_group_first_word() {
    check_eof("shopt -s extglob\n@(ab\necho alive\n", 2);
}

/// A nested `(` inside the group raises the pair depth; the EOF
/// report is unchanged.
#[test]
fn unclosed_group_nested_open() {
    check_eof("shopt -s extglob\necho @(a(b\necho alive\n", 2);
}

/// Complete commands before the failing line have already run (GNU
/// parses and executes incrementally); the report is at the group's
/// line and nothing after it runs.
#[test]
fn unclosed_group_prefix_commands_run() {
    let (stdout, stderr, code) =
        rubash_file("shopt -s extglob\necho before\necho @(ab\necho after\n");
    assert_eq!(code, Some(2), "stderr: {stderr}");
    assert_eq!(stdout, "before\n");
    assert_eq!(
        stderr,
        "case.sh: line 3: unexpected EOF while looking for matching `)'\n"
    );
}

/// Control: extglob OFF keeps the `(`-token error (two lines: the
/// diagnostic and the offending source line) — this shape must not
/// move.
#[test]
fn extglob_off_paren_error_unchanged() {
    for op in OPS {
        let script = format!("echo {op}(ab\necho alive\n");
        let (stdout, stderr, code) = rubash_file(&script);
        assert_eq!(code, Some(2), "script: {script}\nstderr: {stderr}");
        assert_eq!(stdout, "", "script: {script}");
        assert_eq!(
            stderr,
            format!(
                "case.sh: line 1: syntax error near unexpected token `('\n\
                 case.sh: line 1: `echo {op}(ab'\n"
            ),
            "script: {script}"
        );
    }
}

/// Controls: closed groups — on one line, spanning lines, and closed
/// by a `)` on a later line — behave exactly as before (the pattern
/// stays literal in an empty directory and the script runs on).
#[test]
fn closed_group_controls_unchanged() {
    for op in OPS {
        let script = format!("shopt -s extglob\necho {op}(ab)\necho alive\n");
        let (stdout, stderr, code) = rubash_file(&script);
        assert_eq!(code, Some(0), "script: {script}\nstderr: {stderr}");
        assert_eq!(stderr, "", "script: {script}");
        // GNU-verified in this harness's own conditions: the scratch
        // dir holds only case.sh, and `!(ab)` matches every name but
        // `ab` — including case.sh — so the `!` control expands where
        // the other operators stay literal (no file is named `ab`).
        let expected = if op == "!" {
            "case.sh\nalive\n".to_string()
        } else {
            format!("{op}(ab)\nalive\n")
        };
        assert_eq!(stdout, expected, "script: {script}");
    }
    let (stdout, stderr, code) = rubash_file("shopt -s extglob\necho @(a\nb)\necho alive\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "@(a\nb)\nalive\n");
    let (stdout, stderr, code) = rubash_file("shopt -s extglob\necho @(ab\necho x)\necho alive\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "@(ab\necho x)\nalive\n");
    let (stdout, stderr, code) = rubash_file(
        "shopt -s extglob\ncase ab in @(ab)) echo HIT;; *) echo MISS;; esac\necho alive\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "HIT\nalive\n");
}
