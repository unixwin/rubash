//! rubash#2785 — a space (or tab) before the closing paren of a
//! command substitution whose body is a broken conditional must
//! keep the conditional's near token bare: the `)` belongs to the
//! near token only when it is glued to the conditional's end in the
//! source text.
//!
//! GNU parses the `$(` body with the enclosing reader
//! (parse.y:4451 parse_comsub), so for `$( [[ a == b c )` the
//! conditional diagnostic names `c` — the blank between the token
//! and the paren is real source text. rubash's dispatcher trims the
//! extracted body before the body-error report sees it
//! (expand_command_substitution_inner), which destroyed the
//! trailing-blank evidence and made the report extend the near
//! token over the paren exactly as if it had been glued.
//!
//! Expectations measured against GNU bash 5.3.20 (2026-11-08).
//! File mode throughout (both shells exit 2 on a body parse error);
//! the offending-line echo is the stored body fragment in rubash —
//! the documented issue2751 gap — so only the diagnostic lines are
//! pinned, via prefix match plus an exact syntax-line count.

use std::path::PathBuf;
use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_file(tag: &str, script: &str) -> Run {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash2785-{tag}-{}", std::process::id()));
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

fn assert_diagnostic_lines(tag: &str, run: &Run, expected_lines: &[&str]) {
    assert_eq!(run.stdout, "", "{tag}: stdout");
    assert_eq!(run.code, Some(2), "{tag}: rc");
    let expected = expected_lines.join("\n") + "\n";
    assert!(
        run.stderr.starts_with(&expected),
        "{tag}: stderr must start with the GNU diagnostic lines\nwant prefix: {expected:?}\ngot: {:?}",
        run.stderr
    );
    let syntax_lines = run
        .stderr
        .lines()
        .filter(|line| line.contains("syntax error"))
        .count();
    let expected_syntax = expected_lines
        .iter()
        .filter(|line| line.contains("syntax error"))
        .count();
    assert_eq!(
        syntax_lines, expected_syntax,
        "{tag}: exactly the GNU report — no extra syntax line: {:?}",
        run.stderr
    );
}

#[test]
fn issue2785_space_before_paren_keeps_near_bare() {
    let run = run_file("space", "echo $([[ a == b c )\necho AFTER\n");
    assert_diagnostic_lines(
        "space",
        &run,
        &[
            "case.sh: line 1: syntax error in conditional expression: unexpected token `c'",
            "case.sh: line 1: syntax error near `c'",
        ],
    );
}

#[test]
fn issue2785_tab_before_paren_keeps_near_bare() {
    let run = run_file("tab", "echo $([[ a == b\tc )\n");
    assert_diagnostic_lines(
        "tab",
        &run,
        &[
            "case.sh: line 1: syntax error in conditional expression: unexpected token `c'",
            "case.sh: line 1: syntax error near `c'",
        ],
    );
}

#[test]
fn issue2785_glued_paren_still_extends_near() {
    // No blank before the paren: the walk over the closing paren is
    // GNU-true and must not regress.
    let run = run_file("glued", "echo $([[ a == b c)\n");
    assert_diagnostic_lines(
        "glued",
        &run,
        &[
            "case.sh: line 1: syntax error in conditional expression: unexpected token `c'",
            "case.sh: line 1: syntax error near `c)'",
        ],
    );
}

#[test]
fn issue2785_glued_operator_near_still_extends() {
    // The issue2751 glued shapes ride the same report: their bodies
    // carry no trailing blank, so untrimmed body text must judge
    // them glued exactly as before.
    let run = run_file("cond", "echo $([[ a ==]])\n");
    assert_diagnostic_lines(
        "cond",
        &run,
        &[
            "case.sh: line 1: unexpected token `==]]', conditional binary operator expected",
            "case.sh: line 1: syntax error near `==]])'",
        ],
    );
    let run = run_file("cond2", "echo $([[ a == ]])\n");
    assert_diagnostic_lines(
        "cond2",
        &run,
        &[
            "case.sh: line 1: unexpected argument `]]' to conditional binary operator",
            "case.sh: line 1: syntax error near `]])'",
        ],
    );
}

#[test]
fn issue2785_cond_not_at_body_end_keeps_near_bare() {
    let run = run_file("mid", "echo $([[ a == ]] && echo inner)\n");
    assert_diagnostic_lines(
        "mid",
        &run,
        &[
            "case.sh: line 1: unexpected argument `]]' to conditional binary operator",
            "case.sh: line 1: syntax error near `]]'",
        ],
    );
}
