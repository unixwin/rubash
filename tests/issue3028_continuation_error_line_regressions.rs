//! rubash#3028 regressions: after a `\`-continuation, error messages
//! reported the line *before* the continuation instead of the line the
//! command's first word finishes on. (eco-2906 REPORT §6 W1, mvdan/sh
//! execution differential t0004/t0006/t0065/t0187.)
//!
//! GNU mechanism (bash 5.3 source): the reader increments `line_number`
//! for every physical line it crosses, including elided
//! backslash-newline pairs (parse.y:2846 shell_getc, parse.y:2270
//! yy_getc). `read_token_word` captures `simplecmd_lineno = line_number`
//! when a word in command position finishes reading (parse.y:5849-5851),
//! `make_simple_command` stores it as `Simple->line` (command.h:339),
//! and execution stamps it back into `line_number` before running the
//! command (execute_cmd.c:936 SET_LINE_NUMBER). Every diagnostic of
//! that simple command — command-not-found, redirection errors, the
//! `$LINENO` seen by its own expansions — therefore reports the line
//! where the command's FIRST element (word, assignment word, or
//! redirection) finished reading, not where it started.
//!
//! rubash approximated that stamp by counting literal '\n' bytes in the
//! first word's text (`simple_command_first_word_end_line`), which
//! covers quoted multi-line words but is blind to backslash-newline
//! continuations: the lexer elides the pair from the word text, so the
//! count stayed 0 and the stamp kept the word's start line. The fix
//! carries the physical end line from the lexer (Token::end_position,
//! computed from the continuation-join columns the feeder already
//! records) through the parser (CommandNode::first_word_end_line) to
//! the executor stamp.
//!
//! The dangling-connector EOF face (t0065) is the same family on the
//! parser side: the `__RUBASH_PARSE_ERROR_EOF__` renderer reports the
//! carrier node's line + 1 as the end-of-input line, and the fold
//! anchored the carrier on the last stage's START line; a stage whose
//! connector token sits on a later physical line (`foo \` + `|&`)
//! under-reported by the span. The connector arms now stamp the
//! stage's `end_line` from the connector token, and the folds anchor
//! on it.
//!
//! Expectations verified byte-for-byte against GNU Bash 5.3.20
//! script-file probes (self-compiled oracle, LC_ALL=C.UTF-8).

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue3028-{}-{}",
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

/// The issue's minimal form: a continuation inside the command-name
/// word. GNU reports the line the word finishes on (line 2).
#[test]
fn continuation_inside_command_name_reports_word_end_line() {
    check(
        "foo\\\nbar\n",
        "",
        "case.sh: line 2: foobar: command not found\n",
        127,
    );
}

/// t0187 verbatim: the split word is double-quoted; the continuation
/// inside quotes still crosses a physical line for the reader.
#[test]
fn continuation_inside_quoted_name_reports_word_end_line() {
    check(
        "\"foo\\\nbar\"\n",
        "",
        "case.sh: line 2: foobar: command not found\n",
        127,
    );
}

/// t0004 verbatim: the word is continued onto an empty final line;
/// the word finishes reading on line 2 even though nothing follows.
#[test]
fn continuation_to_eof_reports_word_end_line() {
    check(
        "foo\\\n",
        "",
        "case.sh: line 2: foo: command not found\n",
        127,
    );
}

/// A quoted word whose scan is still open when the continuation
/// arrives (`"foo"\` + newline + EOF): the reader crosses the line
/// before the word scan terminates, so GNU reports line 2 — the join
/// abutting the token's end column belongs to the token's own scan,
/// unlike a continuation after a delimiter (`foo \` keeps line 1).
#[test]
fn quoted_word_trailing_continuation_to_eof_reports_line_two() {
    check(
        "\"foo\"\\\n",
        "",
        "case.sh: line 2: foo: command not found\n",
        127,
    );
}

/// t0006 verbatim: `foo\\` is an escaped backslash (part of the name),
/// then a bare continuation backslash joins the empty last line.
#[test]
fn escaped_backslash_plus_continuation_reports_word_end_line() {
    check(
        "foo\\\\\\\n",
        "",
        "case.sh: line 2: foo\\: command not found\n",
        127,
    );
}

/// Two continuations inside one word: the word finishes on line 3.
#[test]
fn double_continuation_inside_name_reports_third_line() {
    check(
        "foo\\\nbar\\\nbaz\n",
        "",
        "case.sh: line 3: foobarbaz: command not found\n",
        127,
    );
}

/// A continuation in the GAP between words does not move the stamp:
/// the first word (`foo`) finished on line 1 (GNU pB shape).
#[test]
fn continuation_between_words_keeps_first_line() {
    check(
        "foo \\\nbar\n",
        "",
        "case.sh: line 1: foo: command not found\n",
        127,
    );
}

/// The first element is the assignment word `x=1`, which finished on
/// line 1; the split command name after it does not move the stamp
/// (GNU simplecmd_lineno is captured at the FIRST element).
#[test]
fn assignment_prefix_captures_stamp_before_split_name() {
    check(
        "x=1 foo\\\nbar\n",
        "",
        "case.sh: line 1: foobar: command not found\n",
        127,
    );
}

/// The assignment word itself is split: it finishes on line 2, so the
/// command (and its not-found name, itself on line 2) reports line 2.
#[test]
fn split_assignment_word_reports_its_end_line() {
    check(
        "x=foo\\\nbar foooo\n",
        "",
        "case.sh: line 2: foooo: command not found\n",
        127,
    );
}

/// A continuation inside an ARGUMENT word does not move the stamp:
/// the command name finished on line 1.
#[test]
fn continuation_inside_argument_keeps_first_line() {
    check(
        "foo ba\\\nr\n",
        "",
        "case.sh: line 1: foo: command not found\n",
        127,
    );
}

/// Both commands of a `;` list on the joined logical line report the
/// physical line they sit on (line 2).
#[test]
fn list_after_continuation_reports_physical_lines() {
    check(
        "foo\\\nbar; bazzz\n",
        "",
        "case.sh: line 2: foobar: command not found\ncase.sh: line 2: bazzz: command not found\n",
        127,
    );
}

/// Pipeline stages keep their own stamps: `foo` finished on line 1,
/// `bar` (after the continued pipe) on line 2.
#[test]
fn pipeline_stages_report_their_own_end_lines() {
    check(
        "foo |\\\nbar\n",
        "",
        "case.sh: line 1: foo: command not found\ncase.sh: line 2: bar: command not found\n",
        127,
    );
}

/// A split command in an `if` condition reports its word-end line;
/// the condition failing makes the whole `if` succeed silently.
#[test]
fn if_condition_split_name_reports_word_end_line() {
    check(
        "if foo\\\nbar; then echo y; fi\n",
        "",
        "case.sh: line 2: foobar: command not found\n",
        0,
    );
}

/// Inside a function body the stamp is the definition-site line where
/// the word finishes (line 3), not the call line and not the word's
/// start line (line 2).
#[test]
fn function_body_split_name_reports_word_end_line() {
    check(
        "f() {\nfoo\\\nbar\n}\nf\n",
        "",
        "case.sh: line 3: foobar: command not found\n",
        127,
    );
}

/// Redirection errors run under the same stamp: the command name
/// finished on line 2, so the failed target open reports line 2 even
/// though the target word itself is also split.
#[test]
fn redirect_error_reports_command_word_end_line() {
    check(
        "foo\\\nbar < /no\\\nsuch\n",
        "",
        "case.sh: line 2: /nosuch: No such file or directory\n",
        1,
    );
}

/// The first element is a redirection whose TARGET word is split:
/// GNU captures simplecmd_lineno when the redirection element
/// finishes reading (after its target), so the stamp is line 2.
#[test]
fn redirect_first_with_split_target_reports_target_end_line() {
    check(
        ">x\\\ny foooo\n",
        "",
        "case.sh: line 2: foooo: command not found\n",
        127,
    );
}

/// t0065 verbatim: `foo \` continues onto a line holding only the
/// `|&` connector, then EOF. GNU's dangling-pipeline syntax error is
/// reported at the end-of-input line (line 3: last physical line + 1).
#[test]
fn dangling_pipe_after_continuation_reports_eof_line() {
    check(
        "foo \\\n\t|&\n",
        "",
        "case.sh: line 3: syntax error: unexpected end of file\n",
        2,
    );
}

/// A dangling `&&` after a split word: the connector sits on line 2
/// and the file ends there, so the EOF error reports line 3.
#[test]
fn dangling_and_or_after_split_word_reports_eof_line() {
    check(
        "foo\\\nbar &&\n",
        "",
        "case.sh: line 3: syntax error: unexpected end of file\n",
        2,
    );
}

/// Non-regression (rubash#2754): a dangling multi-line pipeline
/// already reported the end-of-input line from the last stage.
#[test]
fn dangling_multiline_pipeline_keeps_eof_line() {
    check(
        "echo a |\necho b |\n",
        "",
        "case.sh: line 3: syntax error: unexpected end of file\n",
        2,
    );
}

/// Non-regression: a connector followed by a continuation into EOF —
/// the last token (`&&`) sits on line 1 and the continuation read
/// lands on line 2, which is what GNU reports.
#[test]
fn dangling_connector_then_continuation_eof_keeps_line() {
    check(
        "foo &&\\\n",
        "",
        "case.sh: line 2: syntax error: unexpected end of file\n",
        2,
    );
}

/// Mid-file: the stamp is absolute (physical file line), and the
/// following command still sees the 127 status.
#[test]
fn mid_file_split_name_reports_absolute_line_and_status_flows() {
    check(
        "echo ok\nfoo\\\nbar\necho after=$?\n",
        "ok\nafter=127\n",
        "case.sh: line 3: foobar: command not found\n",
        0,
    );
}

/// `$LINENO` inside the split command's own expansion is the same
/// stamp (execute_cmd.c:936 precedes expansion): `ec` + `ho` finishes
/// the name on line 2, so `$LINENO` expands to 2.
#[test]
fn lineno_inside_split_name_command_is_word_end_line() {
    check("ec\\\nho $LINENO\n", "2\n", "", 0);
}

/// Guard for the investigation that opened this lane: a nameref
/// pointing at an array element (`declare -n ref='a[1]'`, then
/// `ref=hello`) must keep writing exactly `a[1]` — the array stays an
/// array and its other elements survive (verified identical to GNU;
/// the corruption described in the lane brief does not reproduce on
/// this master, and this pin keeps it that way).
#[test]
fn nameref_to_array_element_writes_only_that_element() {
    check(
        "a=(zero one two)\ndeclare -n ref=\"a[1]\"\nref=hello\ndeclare -p a\n",
        "declare -a a=([0]=\"zero\" [1]=\"hello\" [2]=\"two\")\n",
        "",
        0,
    );
}
