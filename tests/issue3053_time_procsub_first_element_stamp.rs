//! rubash#3053 regressions: two first-element line-stamp faces the
//! `first_word_end_line` mechanism of #3028 (PR #3052) does not reach.
//! A simple command's diagnostics and `$LINENO` must report the line
//! where its first element finished reading (GNU: `read_token_word`
//! captures `simplecmd_lineno` at parse.y:5849-5851; the redirection
//! arm captures it at the element's reduction, parse.y:811;
//! execute_cmd.c:936 restamps at execution).
//!
//! (a) `time` prefix: rubash parses `time` as the simple command's
//! own first word, so the provisional stamp is taken from the `time`
//! token and the real first word can no longer restamp it
//! (`push_command_word` restamps only when no element has been
//! recorded yet); stripping the prefix into a `TimeCommand`
//! (parse_loop.rs `time_prefix_from_command`) does not restamp
//! either. The `!` shape was fixed by #3052 because BANG is a token
//! prefix that records no word. The redirect-target correction is
//! likewise blocked while the `time` head word occupies the node.
//!
//! (d) procsub redirect target as first element: plain targets and
//! heredoc delimiters were connected to
//! `correct_first_word_end_for_redirect_target` by #3052; the
//! process-substitution target consumption points were not, so the
//! stamp stays at the redirection operator's provisional line.
//!
//! Expectations verified byte-for-byte against GNU Bash 5.3.20
//! script-file probes (self-compiled oracle, LC_ALL=C.UTF-8).
//! `time`'s own timing report has non-deterministic VALUES even
//! between two GNU runs, so the timed cells either suppress it the
//! GNU way (`TIMEFORMAT=` — an empty format prints no report) or
//! assert the deterministic parts (stdout, exit code, and the
//! diagnostic line, which precedes the report).

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue3053-{}-{}",
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

/// For `time -p` / `time --` cells: GNU appends the POSIX timing
/// report (`real/user/sys`, values vary) after the diagnostic, so
/// assert stdout and exit code exactly and stderr's first line.
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
// (a) `time` prefix + split command name
// -----------------------------------------------------------------

/// The issue's minimal form (with a `TIMEFORMAT=` line suppressing
/// the timing report, so the expected line is 3 in this layout):
/// GNU reports the line the split name finishes on.
#[test]
fn time_split_name_reports_word_end_line() {
    check(
        "TIMEFORMAT=\ntime foo\\\nbar\n",
        "",
        "case.sh: line 3: foobar: command not found\n",
        127,
    );
}

/// The issue's verbatim layout (no TIMEFORMAT line): the diagnostic
/// is stderr's first line, at the word-end line 2.
#[test]
fn time_split_name_reports_word_end_line_bare_layout() {
    check_first_line(
        "time foo\\\nbar\n",
        "",
        "case.sh: line 2: foobar: command not found",
        127,
    );
}

/// `time -p` selects the POSIX report but the same inner stamp.
#[test]
fn time_posix_p_split_name_reports_word_end_line() {
    check_first_line(
        "time -p foo\\\nbar\n",
        "",
        "case.sh: line 2: foobar: command not found",
        127,
    );
}

/// `time --` (TIMEIGN) likewise forces the POSIX report format.
#[test]
fn time_dashdash_split_name_reports_word_end_line() {
    check_first_line(
        "time -- foo\\\nbar\n",
        "",
        "case.sh: line 2: foobar: command not found",
        127,
    );
}

/// `$LINENO` inside the timed split command is the same stamp: the
/// name `ec`+`ho` finishes on line 2, so it expands to 2. GNU's
/// `time -p` report follows on stderr (`real ...` first).
#[test]
fn time_posix_p_split_name_lineno() {
    let (stdout, stderr, code) = rubash_file("time -p ec\\\nho $LINENO\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "2\n");
    assert!(
        stderr.starts_with("real "),
        "expected the POSIX time report on stderr, got: {stderr}"
    );
}

/// With the report suppressed, the timed command's own `$LINENO`
/// is byte-exact: the word finishes on line 3 of this layout.
#[test]
fn time_split_name_lineno_with_empty_timeformat() {
    check("TIMEFORMAT=\ntime ec\\\nho $LINENO\n", "3\n", "", 0);
}

/// Two continuations inside the timed name: it finishes on line 4
/// of this layout.
#[test]
fn time_double_continuation_split_name_reports_end_line() {
    check(
        "TIMEFORMAT=\ntime foo\\\nbar\\\nbaz\n",
        "",
        "case.sh: line 4: foobarbaz: command not found\n",
        127,
    );
}

/// Nested `time time`: both prefixes strip; the inner command
/// still stamps from its own first word.
#[test]
fn time_time_split_name_reports_word_end_line() {
    check(
        "TIMEFORMAT=\ntime time foo\\\nbar\n",
        "",
        "case.sh: line 3: foobar: command not found\n",
        127,
    );
}

/// `time ! foo...`: the BANG inside the timespec does not move the
/// stamp either (and inverts the 127 to 0).
#[test]
fn time_bang_split_name_reports_word_end_line() {
    check(
        "TIMEFORMAT=\ntime ! foo\\\nbar\n",
        "",
        "case.sh: line 3: foobar: command not found\n",
        0,
    );
}

/// The timed command as a pipeline's first stage: the stage's own
/// stamp applies (the pipeline's exit status is `cat`'s 0).
#[test]
fn time_pipeline_first_stage_split_name_reports_word_end_line() {
    check(
        "TIMEFORMAT=\ntime foo\\\nbar | cat\n",
        "",
        "case.sh: line 3: foobar: command not found\n",
        0,
    );
}

/// Under `time`, a redirection as first element whose target word
/// is split: GNU captures the stamp when the redirection element
/// reduces (after its target), so the following word's error
/// reports the target's end line.
#[test]
fn time_redirect_first_split_target_reports_target_end_line() {
    check(
        "TIMEFORMAT=\ntime >x\\\ny foooo\n",
        "",
        "case.sh: line 3: foooo: command not found\n",
        127,
    );
}

/// Chained: under `time`, the first element is an assignment word
/// that is itself split; the `$LINENO` in its own RHS expands under
/// the stamp (the line the assignment word finished on), and the
/// assigned value carries it to the next command.
#[test]
fn time_split_assignment_word_lineno_chain() {
    check(
        "TIMEFORMAT=\ntime x=foo\\\nbar_$LINENO\necho \"x=$x\"\n",
        "x=foobar_3\n",
        "",
        0,
    );
}

/// Guard: a continuation inside an ARGUMENT of the timed command
/// does not move the stamp — the name finished on the `time` line.
#[test]
fn time_argument_split_keeps_first_line() {
    check(
        "TIMEFORMAT=\ntime foo ba\\\nr\n",
        "",
        "case.sh: line 2: foo: command not found\n",
        127,
    );
}

/// Guard: an unsplit timed name reports its own line.
#[test]
fn time_unsplit_name_reports_own_line() {
    check(
        "TIMEFORMAT=\ntime foooo\n",
        "",
        "case.sh: line 2: foooo: command not found\n",
        127,
    );
}

/// Guard (#3052): the `!` prefix shape already restamps at the
/// first word — it must not move.
#[test]
fn bang_split_name_reports_word_end_line_guard() {
    check(
        "! foo\\\nbar\n",
        "",
        "case.sh: line 2: foobar: command not found\n",
        0,
    );
}

// -----------------------------------------------------------------
// (d) procsub redirect target as first element
// -----------------------------------------------------------------

/// `< <(...)` as the first element, the procsub target word split
/// by a continuation: the following word's error reports the
/// target's end line (GNU captures at the redirection's reduction,
/// after the target word was read).
#[test]
fn procsub_input_target_first_split_reports_target_end_line() {
    check(
        "< <(echo a\\\nb) foooo\n",
        "",
        "case.sh: line 2: foooo: command not found\n",
        127,
    );
}

/// Same face through `$LINENO`: the command's own expansion sees
/// the stamp of its first element (the split procsub target).
#[test]
fn procsub_input_target_first_split_lineno() {
    check("< <(echo a\\\nb) echo $LINENO\n", "2\n", "", 0);
}

/// The output form `> >(...)` is the same reduction: a split
/// target word stamps its end line.
#[test]
fn procsub_output_target_first_split_reports_target_end_line() {
    check(
        "> >(c\\\nat) foooo\n",
        "",
        "case.sh: line 2: foooo: command not found\n",
        127,
    );
}

/// Guard: when the procsub redirect is NOT the first element, the
/// stamp belongs to the first word and stays on its line.
#[test]
fn procsub_target_after_word_keeps_word_line() {
    check(
        "foooo < <(echo a\\\nb)\n",
        "",
        "case.sh: line 1: foooo: command not found\n",
        127,
    );
}

/// Guard: the procsub BODY's own split command reports its own
/// word-end line (the body is parsed from the target text with its
/// line provenance; this is the adjacent face, already correct).
#[test]
fn procsub_body_split_name_reports_its_end_line() {
    check(
        "< <(foo\\\nbar) echo done\n",
        "done\n",
        "case.sh: line 2: foobar: command not found\n",
        0,
    );
}
