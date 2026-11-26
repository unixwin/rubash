//! rubash#3235 regressions: a case clause's first simple command,
//! when redirection-first, reports the PATTERN's line — not the line
//! its first element finished on.
//!
//! GNU model (bash 5.3, parse.y): the pattern word is read at command
//! position (the pattern_list newline_list leaves last_read_token at
//! '\n', which reserved_word_acceptable accepts, parse.y:5900-5903),
//! so read_token_word plants `simplecmd_lineno` with the pattern line
//! (parse.y:5850-5851). A redirection element captures the stamp only
//! when it is still unset (parse.y:810-811), so the clause body's
//! first command keeps the pattern line even when its redirect target
//! is split across lines; the command word after the target does not
//! re-plant (WORD is not in reserved_word_acceptable's set), and the
//! plant resets only when that first command completes
//! (`clean_simple_command`, parse.y:838). Wrapper commands in between
//! (groups, if/loop conditions, `time`, `[[ ]]`) consume nothing; a
//! nested case re-plants from its own pattern, so the outer plant
//! never leaks into it. This is the compoundcmd_lineno family the
//! issue names; it is deliberately NOT part of #3209's restamp walk,
//! which stops at case commands.
//!
//! Expectations verified byte-for-byte against GNU Bash 5.3.20
//! script-file probes (self-compiled oracle, LC_ALL=C.UTF-8).
//! The cell count is 15 (12 pattern-line pins + 3 guards).
//!
//! Known residual (NOT pinned): a quoted-newline command word
//! (`a) >x\<newline>y $'a<newline>b'`) still reports the target end
//! line on the fixed tree — the executor's text floor
//! (`simple_command_first_word_end_line`, command_text.rs) takes the
//! max of the restamped value and the first word's newline count, so
//! the pattern line loses to the word's own newline. Fixing that
//! floor touches src/executor/command_text.rs, which open PR #3602
//! is actively editing (same-file guard), so it is recorded here and
//! in the lane REPORT instead of being pinned wrong or fixed under a
//! live PR.

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue3235-{}-{}",
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

/// For cells whose stderr can carry extra output after the diagnostic
/// (the `time` prefix prints its timing report): assert stdout and
/// exit code exactly and stderr's first line.
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
// The issue shape: clause command reports the pattern line
// -----------------------------------------------------------------

/// Minimal form (the issue's own probe): pattern on line 3, the
/// redirect target ends on line 4 — GNU reports line 3.
#[test]
fn clause_redirect_first_reports_pattern_line() {
    check(
        "f() {\ncase a in\na) >x\\\ny foooo ;;\nesac\n}\nf\n",
        "",
        "case.sh: line 3: foooo: command not found\n",
        127,
    );
}

/// The same shape at top level (no enclosing function): pattern on
/// line 2, target ends on line 3 — GNU reports line 2.
#[test]
fn toplevel_clause_redirect_first_reports_pattern_line() {
    check(
        "case a in\na) >x\\\ny foooo ;;\nesac\n",
        "",
        "case.sh: line 2: foooo: command not found\n",
        127,
    );
}

/// A parenthesised pattern `(a)` plants from the same line.
#[test]
fn paren_pattern_reports_pattern_line() {
    check(
        "f() {\ncase a in\n(a) >x\\\ny foooo ;;\nesac\n}\nf\n",
        "",
        "case.sh: line 3: foooo: command not found\n",
        127,
    );
}

/// A multi-alternative pattern plants from its first word's line.
#[test]
fn multi_pattern_reports_pattern_line() {
    check(
        "f() {\ncase a in\na|b) >x\\\ny foooo ;;\nesac\n}\nf\n",
        "",
        "case.sh: line 3: foooo: command not found\n",
        127,
    );
}

/// The second clause plants from ITS OWN pattern line (4), not the
/// first clause's.
#[test]
fn second_clause_reports_its_pattern_line() {
    check(
        "f() {\ncase b in\na) ;;\nb) >x\\\ny foooo ;;\nesac\n}\nf\n",
        "",
        "case.sh: line 4: foooo: command not found\n",
        127,
    );
}

/// The stamp is visible as $LINENO too: the clause command runs with
/// LINENO equal to the pattern line (read back from the redirect
/// target file, as in #3209's LINENO pin).
#[test]
fn clause_first_command_lineno_is_pattern_line() {
    check(
        "f() {\ncase a in\na) >x\\\ny echo $LINENO ;;\nesac\n}\nf\ncat xy\n",
        "3\n",
        "",
        0,
    );
}

// -----------------------------------------------------------------
// Wrappers between the pattern and the command do not consume it
// -----------------------------------------------------------------

/// A brace group around the clause's first command: the plant
/// survives the group and still reports the pattern line.
#[test]
fn group_wrapped_clause_reports_pattern_line() {
    check(
        "f() {\ncase a in\na) { >x\\\ny foooo; }; ;;\nesac\n}\nf\n",
        "",
        "case.sh: line 3: foooo: command not found\n",
        127,
    );
}

/// An if condition as the clause's first command: the condition's
/// redirect-first command reports the pattern line (rc 0 — the
/// failed condition only skips the then-branch).
#[test]
fn if_condition_clause_reports_pattern_line() {
    check(
        "f() {\ncase a in\na) if >x\\\ny foooo; then :; fi ;;\nesac\n}\nf\n",
        "",
        "case.sh: line 3: foooo: command not found\n",
        0,
    );
}

/// A leading `[[ ]]` captures and resets nothing: the plant still
/// applies to the following redirect-first command.
#[test]
fn leading_conditional_clause_reports_pattern_line() {
    check(
        "f() {\ncase a in\na) [[ x ]]; >x\\\ny foooo ;;\nesac\n}\nf\n",
        "",
        "case.sh: line 3: foooo: command not found\n",
        127,
    );
}

/// A `time` prefix does not plant (TIME is a reserved-word token, not
/// a WORD at command position): the pattern line wins.
#[test]
fn time_prefixed_clause_reports_pattern_line() {
    check_first_line(
        "f() {\ncase a in\na) time >x\\\ny foooo ;;\nesac\n}\nf\n",
        "",
        "case.sh: line 3: foooo: command not found",
        127,
    );
}

/// Same, with `time` on the line AFTER the pattern: still the
/// pattern line (3), not the time line (4) or the target end (5).
#[test]
fn time_on_next_line_clause_reports_pattern_line() {
    check_first_line(
        "f() {\ncase a in\na)\ntime >x\\\ny foooo ;;\nesac\n}\nf\n",
        "",
        "case.sh: line 3: foooo: command not found",
        127,
    );
}

/// A nested case re-plants from ITS OWN pattern: the inner clause
/// command reports the inner pattern line (4), never the outer one.
#[test]
fn nested_case_reports_inner_pattern_line() {
    check(
        "f() {\ncase a in\na) case b in\nb) >x\\\ny foooo ;;\nesac ;;\nesac\n}\nf\n",
        "",
        "case.sh: line 4: foooo: command not found\n",
        127,
    );
}

// -----------------------------------------------------------------
// Guards: shapes that must NOT take the pattern line
// -----------------------------------------------------------------

/// Word-first clause command: the command word is read at command
/// position after `)`, so its own end line (4) is the stamp.
#[test]
fn word_first_split_clause_reports_word_end_line() {
    check(
        "f() {\ncase a in\na) foooo\\\nbar ;;\nesac\n}\nf\n",
        "",
        "case.sh: line 4: foooobar: command not found\n",
        127,
    );
}

/// Word-first on the line after the pattern: the word's own line (4).
#[test]
fn word_first_next_line_clause_reports_word_line() {
    check(
        "f() {\ncase a in\na)\nfoooo ;;\nesac\n}\nf\n",
        "",
        "case.sh: line 4: foooo: command not found\n",
        127,
    );
}

/// The SECOND command of a clause body is past the plant: its own
/// redirect target end line (4) is the stamp.
#[test]
fn second_command_clause_reports_its_end_line() {
    check(
        "f() {\ncase a in\na) :; >x\\\ny foooo ;;\nesac\n}\nf\n",
        "",
        "case.sh: line 4: foooo: command not found\n",
        127,
    );
}
