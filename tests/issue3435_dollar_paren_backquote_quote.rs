//! Regression pins for rubash issue #3435: a `$(...)` span scan that is
//! blind to an unbalanced `"` inside a backquote group.
//!
//! GNU bash 5.3 parses a backquote body as its own unit
//! (parse.y:3877 `parse_matched_pair`: for open == close == '`' the
//! grouping branch does NOT parse constructs inside, so quotes and `$`
//! constructs in the body are ordinary characters and a bare backtick
//! ends the body). A body error therefore kills only the substituted
//! child (subst.c `parse_and_execute`): the diagnostic
//! `command substitution: line N: unexpected EOF while looking for
//! matching '"'` is printed, the substitution yields empty, and the
//! enclosing command — and the script — continue with rc 0.
//!
//! Rubash instead rejected the whole input at read time
//! (`rubash: syntax error: unexpected end of file`, rc 2, nothing
//! executed), because the read-time unclosed-quote scan
//! (`quotes_residuals_advance`, src/lexer/continuation.rs) misread the
//! line as an unclosed double quote: its `$(...)` unit skip
//! (`skip_parenthesized_unit_ex`) has no backtick arm, so the `"`
//! inside the backquote leaked into the unit scan and swallowed the
//! true `)`; and its backtick arm does not fire inside double quotes.
//! The same blindness reproduced in two more `$(` span walkers, fixed
//! in this PR by the `in_backtick` discipline of
//! `skip_compound_subscript_span` (src/parser/token_actions.rs:2016):
//! `skip_dollar_paren_body` (src/expand/braces.rs:481, g6 cell) and
//! `skip_dollar_paren` (src/parser/extglob_pattern.rs:307, x1 cell).
//!
//! The read-time quote-scan fix itself lives in
//! src/lexer/continuation.rs (captain-exclusive): the `$(` arm must use
//! the backquote-aware `skip_parenthesized_unit_corrected_ex`, and the
//! backtick arm must fire inside double quotes like the `$(` arm
//! already does. The t2/c cells below fail until that change is applied
//! (see REPORT.md for the exact proposed diff); the g6/x1 cells pass
//! with this PR's own code. Controls: b (no outer quotes — the gate
//! never fired), t1 (the `(` face, falsified — three-way identical),
//! d/e (balanced / quote-free backquote bodies), h (a genuinely
//! unclosed `"` inside `$(`, which stays fatal on both sides).
//!
//! Expectations are the GNU bash 5.3.20 oracle outputs for the lane
//! probe matrix (probe-pincap.sh), compared byte-exact on rc / stdout /
//! stderr with the script file invoked as `rubash case.sh`, so
//! diagnostics carry the bare `case.sh` prefix.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let dir =
            std::env::temp_dir().join(format!("rubash-issue3435-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch dir");
        Scratch(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_case(tag: &str, script: &str) -> (i32, String, String) {
    let scratch = Scratch::new(tag);
    let case_path = scratch.0.join("case.sh");
    fs::write(&case_path, script).expect("write case script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&scratch.0)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("run rubash");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn assert_case(tag: &str, script: &str, want_rc: i32, want_out: &str, want_err: &str) {
    let (rc, out, err) = run_case(tag, script);
    assert_eq!(rc, want_rc, "{tag}: rc\nstdout={out:?}\nstderr={err:?}");
    assert_eq!(out, want_out, "{tag}: stdout\nstderr={err:?}");
    assert_eq!(err, want_err, "{tag}: stderr\nstdout={out:?}");
}

// ---- Fail-first cells: the read-time quote gate (captain fix). ----

#[test]
fn dollar_paren_backquote_unbalanced_dquote_continues() {
    assert_case(
        "t2",
        "echo \"T2:[$(echo `echo \"x`; echo world)]\"\necho AFTER-T2\n",
        0,
        "T2:[\nworld]\nAFTER-T2\n",
        "case.sh: command substitution: line 1: unexpected EOF while looking for matching `\"'\n",
    );
}

#[test]
fn bare_backquote_unbalanced_dquote_continues() {
    assert_case(
        "c",
        "echo \"C:[`echo \"x`\"]\"\necho AFTER-C\n",
        0,
        "C:[]\nAFTER-C\n",
        "case.sh: command substitution: line 1: unexpected EOF while looking for matching `\"'\n",
    );
}

// ---- Fail-first cells: the backquote-blind $( walkers (this PR). ----

#[test]
fn brace_expansion_survives_backquote_unbalanced_dquote() {
    assert_case(
        "g6",
        "echo {$(echo `echo \"q`),x}y\necho AFTER-G6\n",
        0,
        "y xy\nAFTER-G6\n",
        "case.sh: command substitution: line 1: unexpected EOF while looking for matching `\"'\n",
    );
}

#[test]
fn extglob_group_survives_backquote_unbalanced_dquote() {
    assert_case(
        "x1",
        "shopt -s extglob\necho @(ax|$(echo `echo \"q`)ay)\n",
        0,
        "@(ax|ay)\n",
        "case.sh: command substitution: line 2: unexpected EOF while looking for matching `\"'\n",
    );
}

// ---- Controls: faces that already agreed with GNU on base. ----

#[test]
fn unquoted_dollar_paren_backquote_unbalanced_dquote_continues() {
    assert_case(
        "b",
        "echo B:[$(echo `echo \"x`; echo world)]\necho AFTER-B\n",
        0,
        "B:[ world]\nAFTER-B\n",
        "case.sh: command substitution: line 1: unexpected EOF while looking for matching `\"'\n",
    );
}

#[test]
fn dollar_paren_backquote_unbalanced_paren_matches_gnu() {
    assert_case(
        "t1",
        "echo \"T1:[$(echo `echo (`)]\"; echo AFTER-T1\n",
        0,
        "T1:[]\nAFTER-T1\n",
        "case.sh: command substitution: line 1: syntax error near unexpected token `newline'\ncase.sh: command substitution: line 1: `echo ('\n",
    );
}

#[test]
fn dollar_paren_backquote_balanced_dquote_stays_clean() {
    assert_case(
        "d",
        "echo \"D:[$(echo `echo \"x\"`; echo world)]\"\necho AFTER-D\n",
        0,
        "D:[x\nworld]\nAFTER-D\n",
        "",
    );
}

#[test]
fn dollar_paren_backquote_without_quote_stays_clean() {
    assert_case(
        "e",
        "echo \"E:[$(echo `echo x`; echo world)]\"\necho AFTER-E\n",
        0,
        "E:[x\nworld]\nAFTER-E\n",
        "",
    );
}

#[test]
fn dollar_paren_unbalanced_dquote_stays_fatal() {
    assert_case(
        "h",
        "echo \"H:[$(echo \"x)]\"\necho AFTER-H\n",
        2,
        "",
        "case.sh: line 3: unexpected EOF while looking for matching `)'\n",
    );
}
