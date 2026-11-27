//! rubash#977 regressions: the `${...}` verbatim copy arm of
//! `mark_quoted_ifs_from_raw` no longer scans the body with a naive
//! `{`/`}` depth count plus `\` skip. It now delegates the span end to the
//! cook's own braced-parameter copy (`lexer/quotes.rs`
//! copy_braced_parameter_unquoted/_after_dollar →
//! copy_braced_parameter_inner → dolbrace.rs scan_braced_parameter, GNU
//! parse.y:3877 parse_matched_pair P_FIRSTCLOSE|P_DOLBRACE), so `'...'` /
//! `"..."` / `` `...` `` / `$(...)` / nested `${...}` regions are skipped
//! opaquely and a `}` inside them cannot close the span early.
//!
//! The old scan closed `${x:-'}'}` at the `}` inside the quotes, re-walked
//! the tail as outer text, desynced the simulated cook from the real one,
//! and bailed the whole mark pass — quoted IFS chars then split
//! (`"a b"${x:-'}'}$w` printed `<a>` `<bVp>` `<q>` instead of `<a bVp>`
//! `<q>`).
//!
//! Expectations verified byte-for-byte against Git Bash 5.2.37
//! (x86_64-pc-msys GNU build — the same oracle the ticket's audit used;
//! parse_matched_pair's quote handling is stable across 5.2/5.3), script
//! battery at target/issue-suites/results/issue977/p977-class.sh.

use std::process::Command;

/// Run a script FILE in its own scratch directory and return
/// (stdout, stderr, code).
fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-ifsmark977-{}-{}",
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
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn assert_script(script: &str, expected_stdout: &str) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(stdout, expected_stdout, "stdout mismatch");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0), "exit code mismatch");
}

/// Ticket form 1: `}` inside a `'...'` region no longer closes the span.
#[test]
fn dolbrace_body_squote_brace_keeps_quoted_space() {
    assert_script(
        "w=\"p q\"; x=V\nprintf '<%s>\\n' \"a b\"${x:-'}'}$w\n",
        "<a bVp>\n<q>\n",
    );
}

/// Ticket form 2: `}` inside a `$(...)` region no longer closes the span.
#[test]
fn dolbrace_body_comsub_brace_keeps_quoted_space() {
    assert_script(
        "w=\"p q\"; x=V\nprintf '<%s>\\n' \"a b\"${x:-$(echo \"}\")}$w\n",
        "<a bVp>\n<q>\n",
    );
}

/// Ticket form 3: `}` inside a `` `...` `` region no longer closes the span.
#[test]
fn dolbrace_body_backtick_brace_keeps_quoted_space() {
    assert_script(
        "w=\"p q\"; x=V\nprintf '<%s>\\n' \"a b\"${x:-`echo \"}\"`}$w\n",
        "<a bVp>\n<q>\n",
    );
}

/// `}` inside a `"..."` region of the body (same opaque-skip class).
#[test]
fn dolbrace_body_dquote_brace_keeps_quoted_space() {
    assert_script(
        "w=\"p q\"; x=V\nprintf '<%s>\\n' \"a b\"${x:-\"}\"}$w\n",
        "<a bVp>\n<q>\n",
    );
}

/// The quoted `}` is also the default's value when the parameter is unset.
#[test]
fn dolbrace_body_squote_brace_unset_param_uses_default() {
    assert_script(
        "w=\"p q\"; unset u\nprintf '<%s>\\n' \"a b\"${u:-'}'}$w\n",
        "<a b}p>\n<q>\n",
    );
}

/// A nested `${...}` recurses (its own `'`/`"`/`}` are opaque), so the
/// outer span still closes at its own `}`.
#[test]
fn dolbrace_body_nested_dolbrace_keeps_quoted_space() {
    assert_script(
        "w=\"p q\"; x=V\nprintf '<%s>\\n' \"a b\"${x:-${y:-'}'}}$w\n",
        "<a bVp>\n<q>\n",
    );
}

/// P_FIRSTCLOSE: a bare `{` in the body never nests — `${x:-{a,b}}` ends at
/// the `}` after `b` and the trailing `}` is literal outer text.
#[test]
fn dolbrace_body_bare_brace_does_not_nest() {
    assert_script(
        "w=\"p q\"; x=V\nprintf '<%s>\\n' \"a b\"${x:-{a,b}}$w\n",
        "<a bV}p>\n<q>\n",
    );
}

/// A `\}` escape in the body is body text, not the close.
#[test]
fn dolbrace_body_escaped_brace_keeps_quoted_space() {
    assert_script(
        "w=\"p q\"; x=V\nprintf '<%s>\\n' \"a b\"${x:-\\}}$w\n",
        "<a bVp>\n<q>\n",
    );
}

/// `#`/`%` operator bodies take the same opaque scan — a `}` inside their
/// `'...'` pattern text never closed the arm early either.
#[test]
fn dolbrace_operator_pattern_squote_brace_keeps_quoted_space() {
    assert_script(
        "w=\"p q\"; x=V\nprintf '<%s>\\n' \"a b\"${x#'}'}$w\nprintf '<%s>\\n' \"a b\"${x%'}'}$w\n",
        "<a bVp>\n<q>\n<a bVp>\n<q>\n",
    );
}
