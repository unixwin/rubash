//! rubash#880 regressions: three `mark_quoted_ifs_from_raw` bail blind spots.
//!
//! The simulator replays the lexer's quote walk over the raw word and only
//! applies per-char IFS marks when its simulated cook equals the actual
//! cooked word; on mismatch it bails and quoted IFS chars lose protection.
//! Three shapes bailed:
//!
//! 1. backtick `\<newline>` elision was not replayed;
//! 2. dquote-wrapped backtick `\"` was not folded to `"` (GNU subst.c:921-931
//!    string_extract_double_quoted stripdq arm);
//! 3. `$(...)` used a naive paren depth count that closed early on
//!    `$(case x in ...)` pattern parens, so the tail was re-walked as outer
//!    text and any `\`/`"`/`'` there desynced the cook (now case-aware,
//!    arm-for-arm with lexer/quotes.rs copy_dollar_paren_body_raw, GNU
//!    parse.y:4451 + parse.y:3465-3468 via update_command_substitution_case_depth).
//!
//! Every expectation below is byte-verified against WSL GNU Bash 5.3.15
//! (/usr/local/bin/bash, script-file batteries, artifacts under
//! target/probe/880s.sh, 880t.sh, 880u.sh, 880r.sh).

use std::process::Command;

/// Run a script FILE in its own scratch directory and return
/// (stdout, stderr, code).
fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-ifsmark880-{}-{}",
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

/// Face 1: backtick `\<newline>` elision replays, so the quoted space keeps
/// its mark instead of bailing.
#[test]
fn backtick_line_continuation_elision_keeps_quoted_space() {
    assert_script("printf '<%s>\\n' \"a b\"`echo x\\\ny`\n", "<a bxy>\n");
}

/// Face 2: dquote-wrapped backtick `\"` folds to `"` (subst.c:921-931).
#[test]
fn dquote_wrapped_backtick_escaped_quote_folds() {
    assert_script("a=\"`echo \\\"x\\\"`\"\nprintf '[%s]\\n' \"$a\"\n", "[x]\n");
}

/// Face 2 (command-word form): the `\"` fold inside a dquote-wrapped
/// backtick must also replay when the word carries an unquoted expansion
/// sibling — only then does the raw word reach mark_quoted_ifs_from_raw.
/// The simulator used to push DATA_DQUOTE where the cook holds a bare `"`
/// (0x22), so the equality guard bailed on 100% of this shape (PR#898
/// review blocker 1; GNU 5.3.0 prints `<a bxp>` + `<q>`).
#[test]
fn dquote_wrapped_backtick_escaped_quote_folds_in_command_word() {
    assert_script(
        "w=\"p q\"\nprintf '<%s>\\n' \"a b`echo \\\"x\\\"`\"$w\n",
        "<a bxp>\n<q>\n",
    );
}

/// Face 2 locale-quote form: `$"..."` cooks by the same double-quote rules
/// as `"..."` — the command-word walk routed it through
/// preserve_backticks=false, shipping the backtick body's `\"` as
/// DATA_DQUOTE data so the inner echo printed `"x"` (PR#898 review point 4;
/// GNU 5.3.0 prints `<a bxp>` + `<q>`).
#[test]
fn locale_dquote_wrapped_backtick_escaped_quote_folds() {
    assert_script(
        "w=\"p q\"\nprintf '<%s>\\n' $\"a b`echo \\\"x\\\"`\"$w\n",
        "<a bxp>\n<q>\n",
    );
}

/// Face 2 `$(...)`-sibling form: same fold when the unquoted sibling is a
/// command substitution rather than a parameter (PR#898 review point 4;
/// GNU 5.3.0 prints `<a bxp>` + `<q>`).
#[test]
fn dquote_wrapped_backtick_escaped_quote_folds_before_comsub_sibling() {
    assert_script(
        "printf '<%s>\\n' \"a b`echo \\\"x\\\"`\"$(echo p q)\n",
        "<a bxp>\n<q>\n",
    );
}

/// Face 1 nested form: inside `$(...)`, a `\`` in a nested backtick body
/// must be consumed as a `\x` pair — single-stepping let the escaped
/// backtick close the span early, the leftover `)` then closed the `$()`,
/// and the tail re-walked as outer text desynced the cook into a bail
/// (PR#898 review blocker 2; GNU 5.3.0 prints `<a b)>` + `<$y>`).
#[test]
fn comsub_nested_backtick_escaped_backtick_pairs_through() {
    assert_script(
        "printf '<%s>\\n' \"a b\"$(echo `echo \\`echo \\\\)\\`` \\$y)\n",
        "<a b)>\n<$y>\n",
    );
}

/// Face 3a: `$(case ...)` with `\$` in the body no longer early-closes.
#[test]
fn case_comsub_with_escaped_dollar_keeps_quoted_space() {
    assert_script(
        "printf '<%s>\\n' \"a b\"$(case x in x) echo \\$y;; esac)\n",
        "<a b$y>\n",
    );
}

/// Face 3b: `$(case ...)` with `\\` in the body no longer early-closes.
#[test]
fn case_comsub_with_escaped_backslash_keeps_quoted_space() {
    assert_script(
        "printf '<%s>\\n' \"a b\"$(case x in x) echo \\\\;; esac)\n",
        "<a b\\>\n",
    );
}

/// Face 3c: `$(case ...)` with an inner double-quoted blank no longer
/// early-closes (the comsub result still splits, glued to the quoted span).
#[test]
fn case_comsub_with_inner_dquotes_keeps_quoted_space() {
    assert_script(
        "printf '<%s>\\n' \"a b\"$(case x in x) echo \"h i\";; esac)\n",
        "<a bh>\n<i>\n",
    );
}

/// Face 3d: `$(case ...)` with an inner single-quoted blank, same as 3c.
#[test]
fn case_comsub_with_inner_squotes_keeps_quoted_space() {
    assert_script(
        "printf '<%s>\\n' \"a b\"$(case x in x) echo 'h i';; esac)\n",
        "<a bh>\n<i>\n",
    );
}
