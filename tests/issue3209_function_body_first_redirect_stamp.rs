//! rubash#3209 regressions: a function body's first simple command
//! whose first element is a redirection inherits the line of the
//! function DEFINITION (the bare `name()` word), not the line its
//! first element finished on.
//!
//! GNU model (bash 5.3, parse.y): the function name word is read at
//! command position, so `read_token_word` plants `simplecmd_lineno`
//! with the name's line (parse.y:5849-5851, gated by
//! `command_token_position`, parse.y:3157-3161). The redirection
//! element captures the stamp only when it is still unset
//! (parse.y:810-811 `if (simplecmd_lineno == -1)`), and the first
//! simple command to COMPLETE consumes the plant
//! (`clean_simple_command`, parse.y:838). Wrapper commands in
//! between (groups, if/loop conditions, pipelines, time, `!`,
//! background) do not consume it; a completed `[[ ]]` / `(( ))`
//! does not either, because those never capture or reset the stamp.
//! The `function` keyword forms plant nothing: the name follows the
//! FUNCTION token, so it is never read at command position.
//!
//! A bare newline after a redirection TERMINATES the simple command
//! in GNU, so every split below is a backslash continuation inside
//! one logical command; an earlier probe series that used bare
//! newlines measured separate commands and was discarded.
//!
//! Expectations verified byte-for-byte against GNU Bash 5.3.20
//! script-file probes (self-compiled oracle, LC_ALL=C.UTF-8).
//! The case-clause shape is deliberately NOT pinned here: a clause
//! command's stamp is captured from the pattern token (a distinct
//! compound-command mechanism) — probed and recorded in the wave
//! report as a residual.
//!
//! Reconstruction note (2026-11-10): the tail of this file — from
//! the middle of `nested_function_reports_inner_definition_line`
//! to EOF — was lost when a truncated tool output was written back
//! to disk during the write-wall episode. It has been reconstructed
//! from the wave's surviving GNU probe runners
//! (`pins3209/run_gnu2.py`, `pins3209/run_gnu3.py`), with every
//! expectation re-measured against the GNU oracle on the day of
//! the reconstruction. Seven further matrix shapes (keyword forms,
//! a second command, an assignment-completed command, a word-first
//! split, a pipeline second stage, a redirect-only first command)
//! were measured but deliberately NOT pinned here: on both the
//! base and the fixed tree rubash reports line 1 for them where
//! GNU reports line 2 — a distinct flattening of the first body
//! command's stamp inside definition capture, unchanged by this
//! wave's change and outside its mechanism. Their measured values
//! are recorded in the wave report addendum. The cell count is 24.

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue3209-{}-{}",
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

/// For cells whose stderr can carry extra asynchronous output (a
/// process substitution body racing the diagnostic): assert stdout
/// and exit code exactly and stderr's first line.
fn check_first_line(
    script: &str,
    expected_stdout: &str,
    expected_first_line: &str,
    expected_code: i32,
) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(
        code,
        Some(expected_code),
        "script: {script:?}\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert_eq!(stdout, expected_stdout, "script: {script:?}");
    assert_eq!(
        stderr.lines().next(),
        Some(expected_first_line),
        "script: {script:?}\nstderr: {stderr}"
    );
}

// -----------------------------------------------------------------
// The plant: first body command is redirection-first
// -----------------------------------------------------------------

/// Minimal form: the redirect target is split across lines by a
/// continuation; GNU reports the definition line (1), not the
/// target's end line (2).
#[test]
fn split_target_reports_definition_line() {
    check(
        "f() { >x\\\ny foooo; }\nf\n",
        "",
        "case.sh: line 1: foooo: command not found\n",
        127,
    );
}

/// The definition line is above the body command entirely.
#[test]
fn multiline_body_split_target_reports_definition_line() {
    check(
        "f() {\n >x\\\ny foooo; }\nf\n",
        "",
        "case.sh: line 1: foooo: command not found\n",
        127,
    );
}

/// Same, with the body command on its own line and a closing brace
/// line (the ordinary multi-line function shape).
#[test]
fn brace_body_own_line_split_target_reports_definition_line() {
    check(
        "f() {\n >x\\\ny foooo\n}\nf\n",
        "",
        "case.sh: line 1: foooo: command not found\n",
        127,
    );
}

/// Two redirections; the FIRST target is the split one.
#[test]
fn two_redirects_split_first_target_reports_definition_line() {
    check(
        "f() { >x\\\ny >z foooo; }\nf\n",
        "",
        "case.sh: line 1: foooo: command not found\n",
        127,
    );
}

/// A split process-substitution target: the target (the whole
/// `<(...)` span) ends on line 2; GNU still reports line 1.
#[test]
fn split_procsub_target_reports_definition_line() {
    check_first_line(
        "f() { > >(cat \\\na) foooo; }\nf\n",
        "",
        "case.sh: line 1: foooo: command not found",
        127,
    );
}

/// The split header form: the name sits on line 1, the brace opens
/// on line 2 — the NAME's line is the definition line.
#[test]
fn split_header_reports_name_line() {
    check(
        "f()\n{ >x\\\ny foooo; }\nf\n",
        "",
        "case.sh: line 1: foooo: command not found\n",
        127,
    );
}

/// The body's first command executes with `$LINENO` equal to the
/// definition line (the value lands in the redirect target file
/// `xy`, read back after the call).
#[test]
fn body_first_command_lineno_is_definition_line() {
    check("f() { >x\\\ny echo $LINENO; }\nf\ncat xy\n", "1\n", "", 0);
}

// -----------------------------------------------------------------
// Wrappers between the header and the command do not consume it
// -----------------------------------------------------------------

#[test]
fn group_first_reports_definition_line() {
    check(
        "f() { { >x\\\ny foooo; }; }\nf\n",
        "",
        "case.sh: line 1: foooo: command not found\n",
        127,
    );
}

#[test]
fn if_condition_reports_definition_line() {
    check(
        "f() { if >x\\\ny foooo; then :; fi; }\nf\n",
        "",
        "case.sh: line 1: foooo: command not found\n",
        0,
    );
}

#[test]
fn subshell_body_form_reports_definition_line() {
    check(
        "f() ( >x\\\ny foooo )\nf\n",
        "",
        "case.sh: line 1: foooo: command not found\n",
        127,
    );
}

#[test]
fn while_condition_reports_definition_line() {
    check(
        "f() { while >x\\\ny foooo; do break; done; }\nf\n",
        "",
        "case.sh: line 1: foooo: command not found\n",
        0,
    );
}

#[test]
fn for_body_reports_definition_line() {
    check(
        "f() { for i in 1; do >x\\\ny foooo; done; }\nf\n",
        "",
        "case.sh: line 1: foooo: command not found\n",
        127,
    );
}

#[test]
fn pipeline_first_stage_reports_definition_line() {
    check(
        "f() { >x\\\ny foooo | cat; }\nf\n",
        "",
        "case.sh: line 1: foooo: command not found\n",
        0,
    );
}

#[test]
fn and_or_first_reports_definition_line() {
    check(
        "f() { >x\\\ny foooo && echo ok; }\nf\n",
        "",
        "case.sh: line 1: foooo: command not found\n",
        127,
    );
}

#[test]
fn background_first_reports_definition_line() {
    check(
        "f() { >x\\\ny foooo & }\nf\n",
        "",
        "case.sh: line 1: foooo: command not found\n",
        0,
    );
}

#[test]
fn inverted_first_reports_definition_line() {
    check(
        "f() { ! >x\\\ny foooo; }\nf\n",
        "",
        "case.sh: line 1: foooo: command not found\n",
        0,
    );
}

#[test]
fn coproc_first_reports_definition_line() {
    check(
        "f() { coproc >x\\\ny foooo; }\nf\n",
        "",
        "case.sh: line 1: foooo: command not found\n",
        0,
    );
}

/// A leading `[[ ]]` captures and resets nothing: the plant still
/// applies to the next command.
#[test]
fn leading_conditional_does_not_consume() {
    check(
        "f() { [[ x ]]; >x\\\ny foooo; }\nf\n",
        "",
        "case.sh: line 1: foooo: command not found\n",
        127,
    );
}

/// Same for a leading `(( ))`.
#[test]
fn leading_arithmetic_does_not_consume() {
    check(
        "f() { ((x=1)); >x\\\ny foooo; }\nf\n",
        "",
        "case.sh: line 1: foooo: command not found\n",
        127,
    );
}

/// Composition with the `time` prefix (#3053's face): the timed
/// command is redirection-first; the definition line (2 in this
/// layout) wins over both the time token and the target end (3).
#[test]
fn time_wrapped_first_reports_definition_line() {
    check(
        "TIMEFORMAT=\nf() { time >x\\\ny foooo; }\nf\n",
        "",
        "case.sh: line 2: foooo: command not found\n",
        127,
    );
}

/// A nested bare definition: the INNER name's line governs the
/// inner body (its split target ends on line 3).
#[test]
fn nested_function_reports_inner_definition_line() {
    check(
        "f() {\n g() { >x\\\ny foooo; }; g; }\nf\n",
        "",
        "case.sh: line 2: foooo: command not found\n",
        127,
    );
}

// -----------------------------------------------------------------
// Guards: shapes that must NOT inherit the definition line
// -----------------------------------------------------------------

/// Outside any definition there is no plant: a top-level
/// redirection-first command reports its target end line (2).
#[test]
fn toplevel_split_target_reports_its_end_line() {
    check(
        ">x\\\ny foooo\n",
        "",
        "case.sh: line 2: foooo: command not found\n",
        127,
    );
}

/// The mirror guard: the command NAME splits, but the first
/// element (the redirection) is what carries the stamp — the
/// definition line (1) still wins.
#[test]
fn split_name_after_redirect_keeps_definition_line() {
    check(
        "f() { >x foooo\\\nbar; }\nf\n",
        "",
        "case.sh: line 1: foooobar: command not found\n",
        127,
    );
}

/// A heredoc redirection as the first element: the split command
/// name still reports the definition line (1) — the heredoc body
/// lines do not move the stamp.
#[test]
fn heredoc_split_name_reports_definition_line() {
    check(
        "f() { <<EOF foo\\\nbar\nEOF\n}\nf\n",
        "",
        "case.sh: line 1: foobar: command not found\n",
        127,
    );
}
