//! rubash#3210 regressions: a process-substitution body whose first
//! command name is split by a backslash-newline reports the wrong
//! diagnostic line — GNU reports the line the split name ENDS on
//! (body-relative), rubash reports the line it STARTS on.
//!
//! The defect is in the SEED, not the body text: the outer parse
//! stamps the split word's end line correctly (#3052,
//! `Token::end_position`, and the node carries it as
//! `first_word_end_line`), but every procsub spawn site passed
//! `cmd.line` (the command's START line) as the body seed. The
//! re-print folding the continuation is harmless — it is equivalent
//! to GNU's child-side no-bump for `\<newline>` inside a word
//! (parse.y:5377-5381). The fix seeds each spawn with
//! `procsub_body_seed_line` (the first-element end line,
//! execute_cmd.c:936 SET_LINE_NUMBER).
//!
//! The function-body face (Residual B, same issue) is a different
//! loss point: the scanner FOLDS a single-logical-line `{...}`
//! group into one token, and the re-lex sees the joined value with
//! no newlines, so every body command reports the group's first
//! line. The fix refuses the fold when a continuation join falls
//! inside the group span (scanner.rs `{` arm), so the parser takes
//! the slice path with feeder-stamped tokens.
//!
//! Facecheck vs #1184-f4 (construction-line seed): DIFFERENT point.
//! #1184-f4 fixed the body's line BASE (seed = construction line);
//! #3210 is the word's END line WITHIN the body. The f4 fix and
//! #3052 were both already in place on the P5 fail-first base
//! (`f17644d7`) and d8 still reported line 1.
//!
//! Expectations verified byte-for-byte against GNU Bash 5.3.20
//! script-file probes (self-compiled oracle, LC_ALL=C.UTF-8).
//! Input procsub: reported = construction line + body-relative end
//! line - 1 (d8: 1+2-1=2; c_cat3: 1+3-1=3; a_1184f4: 5+1-1=5).
//! Output procsub multi-command bodies follow the same rule
//! (o1: 1; o2: 2; o3: 3); the output split-word body is the GNU
//! quirk — it reports the construction line (out_split: 1, not 2).

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue3210-{}-{}",
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

fn check(script: &str, expected_stdout: &str, expected_stderr: &str, expected_code: i32) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(
        code,
        Some(expected_code),
        "script: {script:?}\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert_eq!(stdout, expected_stdout, "script: {script:?}");
    assert_eq!(stderr, expected_stderr, "script: {script:?}");
}

// -----------------------------------------------------------------
// Input process substitution `<(...)`: the defect cells
// -----------------------------------------------------------------

/// The issue's minimal form (was #3053 pin d8,摘格 for the
/// diagnostic-line-source face): the split name `foo\<newline>bar`
/// ends on body line 2; GNU reports line 2, rubash reported line 1.
#[test]
fn procsub_body_split_name_reports_its_end_line() {
    check(
        "< <(foo\\\nbar) echo done\n",
        "done\n",
        "case.sh: line 2: foobar: command not found\n",
        0,
    );
}

/// Distinguisher: a NON-split word on body line 2. If this cell is
/// green while the split-word cell is red, only the continuation
/// join is lost; if this is red too, the body line base itself is
/// broken.
#[test]
fn procsub_body_second_command_reports_body_line() {
    check(
        "cat < <(echo a\nfoobar)\n",
        "a\n",
        "case.sh: line 2: foobar: command not found\n",
        0,
    );
}

/// Third body line, same family: the body-relative line must count
/// up through the re-print.
#[test]
fn procsub_body_third_command_reports_body_line() {
    check(
        "cat < <(echo a\necho b\nfoobar)\n",
        "a\nb\n",
        "case.sh: line 3: foobar: command not found\n",
        0,
    );
}

/// Control: an error on body line 1 reports line 1 (the seed is the
/// construction line, #1184-f4's face — must not move).
#[test]
fn procsub_body_error_on_first_line_reports_one() {
    check(
        "cat < <(foobar)\n",
        "",
        "case.sh: line 1: foobar: command not found\n",
        0,
    );
}

// -----------------------------------------------------------------
// Output process substitution `>(...)`
// -----------------------------------------------------------------

/// Single-line body, output form: line 1.
#[test]
fn procsub_output_body_error_on_first_line_reports_one() {
    check(
        "echo done > >(foobar)\n",
        "",
        "case.sh: line 1: foobar: command not found\n",
        0,
    );
}

/// Second body line, output form: line 2. (`echo done` writes into
/// the pipe, so the only stdout is the body's own `echo a`.)
#[test]
fn procsub_output_body_second_command_reports_body_line() {
    check(
        "echo done > >(echo a\nfoobar)\n",
        "a\n",
        "case.sh: line 2: foobar: command not found\n",
        0,
    );
}

/// Third body line, output form: line 3.
#[test]
fn procsub_output_body_third_command_reports_body_line() {
    check(
        "echo done > >(echo a\necho b\nfoobar)\n",
        "a\nb\n",
        "case.sh: line 3: foobar: command not found\n",
        0,
    );
}

/// The GNU quirk: the output form's split-word body reports the
/// CONSTRUCTION line (1), not the word's end line (2). This cell
/// pins that asymmetry — it must stay line 1 while the input
/// form's identical body reports line 2.
#[test]
fn procsub_output_body_split_name_reports_construction_line() {
    check(
        "echo done > >(foo\\\nbar)\n",
        "",
        "case.sh: line 1: foobar: command not found\n",
        0,
    );
}

// -----------------------------------------------------------------
// Command substitution `$(...)`: negative controls (must not drift)
// -----------------------------------------------------------------

/// Comsub split-word body: GNU reports the word's end line (2),
/// same shape as the input-procsub defect cell. GNU exits 127 here
/// (the body command is not found); the original rc=0 expectation
/// was a pin bug, corrected against the oracle.
#[test]
fn comsub_body_split_name_reports_its_end_line() {
    check(
        "x=$(foo\\\nbar)\n",
        "",
        "case.sh: line 2: foobar: command not found\n",
        127,
    );
}

/// Comsub second body command: GNU reports line 3 (its line base
/// is the enclosing command's end line, not the construction
/// line — a different rule from procsub, pinned here so the
/// procsub fix cannot drag it).
#[test]
fn comsub_body_second_command_reports_body_line() {
    check(
        "x=$(echo a\nfoobar)\necho done\n",
        "done\n",
        "case.sh: line 3: foobar: command not found\n",
        0,
    );
}

// -----------------------------------------------------------------
// Function-definition bodies (#3210 + Residual B): the split-name
// diagnostic line is the name's END line, except where GNU's
// simplecmd_lineno plant wins (bare-name paren form, redirect-first
// first command — pinned by the issue3209 stem). The plant is
// narrow: keyword forms plant nothing (the name follows FUNCTION),
// and it is consumed by the body's first simple command, so a
// second command, an assignment-first command, a pipeline second
// stage, or a redirect-only first command all report the natural
// first-element end line.
// -----------------------------------------------------------------

/// Residual B (1): `function` keyword form plants nothing — the
/// redirect-first first command reports the redirect target's end
/// line (2), not the definition line (1).
#[test]
fn fn_body_keyword_split_target_reports_end_line() {
    check(
        "function f { >x\\\ny foooo; }\nf\n",
        "",
        "case.sh: line 2: foooo: command not found\n",
        127,
    );
}

/// Residual B (2): `function f()` — same, keyword + parentheses.
#[test]
fn fn_body_keyword_paren_split_target_reports_end_line() {
    check(
        "function f() { >x\\\ny foooo; }\nf\n",
        "",
        "case.sh: line 2: foooo: command not found\n",
        127,
    );
}

/// Residual B (3): second command — the plant was consumed by the
/// first (`:`), so the redirect-first second command reports its
/// target's end line (2).
#[test]
fn fn_body_second_command_split_target_reports_end_line() {
    check(
        "f() { :; >x\\\ny foooo; }\nf\n",
        "",
        "case.sh: line 2: foooo: command not found\n",
        127,
    );
}

/// Residual B (4): an assignment first command consumes the plant
/// the same way.
#[test]
fn fn_body_assignment_first_split_target_reports_end_line() {
    check(
        "f() { x=1; >x\\\ny foooo; }\nf\n",
        "",
        "case.sh: line 2: foooo: command not found\n",
        127,
    );
}

/// Residual B (5): the issue's word-split form in a function body —
/// the split name reports its end line (2).
#[test]
fn fn_body_split_word_reports_end_line() {
    check(
        "f() { foooo\\\nbar; }\nf\n",
        "",
        "case.sh: line 2: foooobar: command not found\n",
        127,
    );
}

/// Residual B (6): pipeline second stage — the first stage consumed
/// the plant, so the redirect-first second stage reports its
/// target's end line (2).
#[test]
fn fn_body_pipeline_second_stage_split_target_reports_end_line() {
    check(
        "f() { echo hi | >x\\\ny foooo; }\nf\n",
        "",
        "case.sh: line 2: foooo: command not found\n",
        127,
    );
}

/// Residual B (7): a redirect-only first command does not plant or
/// consume — the split word after it reports its own end line (2).
#[test]
fn fn_body_redirect_only_then_split_word_reports_end_line() {
    check(
        "f() { >x; foo\\\nbar; }\nf\n",
        "",
        "case.sh: line 2: foobar: command not found\n",
        127,
    );
}

// -----------------------------------------------------------------
// Design-decision guards: a split word reports its end line in
// every other body position (second command, pipeline stage,
// after an assignment, keyword forms, multi-line bodies, nested
// definitions). These pin the "plant/reset semantics, not restamp
// range" design — widening the #3209 restamp to these shapes would
// be wrong.
// -----------------------------------------------------------------

#[test]
fn fn_body_second_command_split_word_reports_end_line() {
    check(
        "f() { :; foooo\\\nbar; }\nf\n",
        "",
        "case.sh: line 2: foooobar: command not found\n",
        127,
    );
}

#[test]
fn fn_body_pipeline_split_word_reports_end_line() {
    check(
        "f() { echo hi | foooo\\\nbar; }\nf\n",
        "",
        "case.sh: line 2: foooobar: command not found\n",
        127,
    );
}

#[test]
fn fn_body_redirect_only_split_word_reports_end_line() {
    check(
        "f() { >x; foooo\\\nbar; }\nf\n",
        "",
        "case.sh: line 2: foooobar: command not found\n",
        127,
    );
}

#[test]
fn fn_body_assignment_split_word_reports_end_line() {
    check(
        "f() { x=1; foooo\\\nbar; }\nf\n",
        "",
        "case.sh: line 2: foooobar: command not found\n",
        127,
    );
}

#[test]
fn fn_body_keyword_split_word_reports_end_line() {
    check(
        "function f { foooo\\\nbar; }\nf\n",
        "",
        "case.sh: line 2: foooobar: command not found\n",
        127,
    );
}

#[test]
fn fn_body_keyword_paren_split_word_reports_end_line() {
    check(
        "function f() { foooo\\\nbar; }\nf\n",
        "",
        "case.sh: line 2: foooobar: command not found\n",
        127,
    );
}

#[test]
fn fn_body_multiline_split_word_reports_end_line() {
    check(
        "f() {\n foooo\\\nbar\n}\nf\n",
        "",
        "case.sh: line 3: foooobar: command not found\n",
        127,
    );
}

#[test]
fn fn_body_nested_split_word_reports_end_line() {
    check(
        "f() {\n g() { foooo\\\nbar; }; g; }\nf\n",
        "",
        "case.sh: line 3: foooobar: command not found\n",
        127,
    );
}

/// `$LINENO` agrees with the diagnostic line for a split word.
#[test]
fn fn_body_split_word_lineno_agrees_with_diagnostic() {
    check(
        "f() { foooo\\\nbar; echo $LINENO; }\nf\n",
        "2\n",
        "case.sh: line 2: foooobar: command not found\n",
        0,
    );
}

/// `$LINENO` agrees for the second-command redirect-first shape:
/// the plant is consumed, so the echo reports 2 (file `xy`).
#[test]
fn fn_body_second_command_lineno_is_end_line() {
    check(
        "f() { :; >x\\\ny echo $LINENO; }\nf\ncat xy\n",
        "2\n",
        "",
        0,
    );
}

// -----------------------------------------------------------------
// Process-substitution seed guards (#3210 procsub site): the seed
// is the enclosing command's FIRST-ELEMENT end line (GNU
// execute_cmd.c:936 SET_LINE_NUMBER(Simple->line)), not its start
// line — so a split outer first word and a multi-line target move
// the seed, while a word-first `exec` stays on its own line.
// -----------------------------------------------------------------

/// Word-form procsub with a split outer first word: the seed is the
/// word's end line (2).
#[test]
fn procsub_word_form_split_outer_word_reports_end_line() {
    check(
        "ca\\\nt <(foo\\\nbar)\n",
        "",
        "case.sh: line 2: foobar: command not found\n",
        0,
    );
}

/// A split-word body followed by another body command: the seed is
/// the TARGET's end line (3 — the `)` closes on line 3), and the
/// body-relative lines count up from there (3, 4).
#[test]
fn procsub_body_split_name_then_command_reports_lines() {
    check(
        "< <(foo\\\nbar\nbaz) echo done\n",
        "done\n",
        "case.sh: line 3: foobar: command not found\ncase.sh: line 4: baz: command not found\n",
        0,
    );
}

// -----------------------------------------------------------------
// Bare brace groups: the same fold is the loss point (the group is
// folded and re-lexed, same as a function body), so the same
// refusal covers them. `if` bodies and subshells never fold and
// were already green (probed 2026-11-11).
// -----------------------------------------------------------------

/// Bare group, split word as the only command.
#[test]
fn bare_group_split_word_reports_end_line() {
    check(
        "{ foooo\\\nbar; }\n",
        "",
        "case.sh: line 2: foooobar: command not found\n",
        127,
    );
}

/// Bare group, split word after a completed command.
#[test]
fn bare_group_second_command_split_word_reports_end_line() {
    check(
        "{ echo hi; foooo\\\nbar; }\n",
        "hi\n",
        "case.sh: line 2: foooobar: command not found\n",
        127,
    );
}

// -----------------------------------------------------------------
// eval form: the eval'd text flows through the same feeder
// (tokenize_with_heredocs -> GroupScanFeeder), so the same fold
// refusal covers it.
// -----------------------------------------------------------------

/// A function defined inside eval with a split-word body.
#[test]
fn eval_defined_function_split_word_reports_end_line() {
    check(
        "eval 'f() { foooo\\\nbar; }'\nf\n",
        "",
        "case.sh: line 2: foooobar: command not found\n",
        127,
    );
}
