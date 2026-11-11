//! Issue unixwin/rubash#2653: an alias whose value contains a literal
//! newline is registered (the `alias` query prints it verbatim) but is
//! never expanded at invocation — `foo2: command not found` — whenever
//! the definition is read while the alias table is already non-empty.
//!
//! GNU contract: the alias value is stored verbatim (alias.c:108
//! `add_alias` — `savestring(value)`, no line splitting) and expansion
//! pushes the value back into the reader (`parse.y:3249
//! alias_expand_token` -> `parse.y:2046 push_string`, called from
//! `parse.y:5761 read_token_word`), where it is re-read as ordinary
//! input — an embedded newline is a command terminator in the re-read
//! stream, so `foo2` runs `echo three` and then `echo four`. GNU's
//! reader consumes ONE complete command at a time and executes it
//! before reading the next (evalstring.c parse_and_execute), so a
//! definition takes effect for every command read after it.
//!
//! Defect mechanism (diagnosed on master f7fed212): the grouped
//! driver's alias-live gather arm feeds the streaming alias expander
//! (lexer/alias_stream.rs AliasStreamScanner) line by line. A rescan
//! park (an unterminated word/quote span) arms a retry deadline of
//! `buffer_len + max(span/16, 2048)`; once the first alias exists, a
//! multi-line alias DEFINITION parks the scanner on its open quote and
//! the 2048-char floor keeps it parked for the rest of a short script —
//! the gather never sees the closing line's verdict, swallows the
//! remainder of the file into ONE read unit, and batch-expands that
//! unit against the table as of the unit's start. Definitions inside
//! the unit register (queries see them), but invocations inside the
//! same unit can no longer expand — exactly GNU's same-unit rule
//! applied to a unit GNU would never have formed.
//!
//! Every expectation below is byte-verified against GNU bash 5.3.20
//! run on the same script file. Defect cells are red on master; the
//! control cells are green on master and must stay green.

use std::path::PathBuf;
use std::process::Command;

fn run_script(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash2653-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let file: PathBuf = dir.join("case.sh");
    std::fs::write(&file, script).expect("write script");
    let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&file)
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

fn assert_clean(tag: &str, script: &str, expected_stdout: &str) {
    let (stdout, stderr, code) = run_script(tag, script);
    assert_eq!(stdout, expected_stdout, "{tag}: stdout");
    assert_eq!(stderr, "", "{tag}: stderr must be empty");
    assert_eq!(code, Some(0), "{tag}: exit code");
}

// ---------------------------------------------------------------------
// Defect cells (red on master, green after the fix)
// ---------------------------------------------------------------------

/// The issue's reproducer (upstream alias2.sub shape): a single-line
/// alias first, then the multi-line definition, then both invoked.
#[test]
fn multiline_value_expands_after_prior_alias() {
    assert_clean(
        "core",
        "shopt -s expand_aliases\n\
         alias foo1='echo one; echo two'\n\
         alias foo2='echo three\n\
         echo four'\n\
         foo1\n\
         foo2\n",
        "one\ntwo\nthree\nfour\n",
    );
}

/// The prior alias is never invoked: registration order alone (the
/// table being live while the multi-line definition is gathered) is
/// the trigger.
#[test]
fn multiline_value_expands_without_prior_invocation() {
    assert_clean(
        "noinv",
        "shopt -s expand_aliases\n\
         alias foo1='echo one; echo two'\n\
         alias foo2='echo three\n\
         echo four'\n\
         foo2\n",
        "three\nfour\n",
    );
}

/// A value spanning three physical lines (two embedded newlines).
#[test]
fn three_line_value_expands() {
    assert_clean(
        "threeline",
        "shopt -s expand_aliases\n\
         alias foo1='echo one; echo two'\n\
         alias foo2='echo three\n\
         echo four\n\
         echo five'\n\
         foo2\n",
        "three\nfour\nfive\n",
    );
}

/// The damage is not limited to the multi-line alias itself: an alias
/// defined AFTER the multi-line definition (while the gather is still
/// swallowing the file into one unit) must also expand.
#[test]
fn alias_defined_after_multiline_definition_expands() {
    assert_clean(
        "after",
        "shopt -s expand_aliases\n\
         alias foo1='echo one; echo two'\n\
         alias foo2='echo three\n\
         echo four'\n\
         alias foo3='echo F3'\n\
         foo3\n\
         foo2\n",
        "F3\nthree\nfour\n",
    );
}

/// Two multi-line definitions in a row: the first (read with an empty
/// table) already works on master; the second must work too.
#[test]
fn second_multiline_definition_expands() {
    assert_clean(
        "twoml",
        "shopt -s expand_aliases\n\
         alias fooA='echo A1\n\
         echo A2'\n\
         alias fooB='echo B1\n\
         echo B2'\n\
         fooA\n\
         fooB\n",
        "A1\nA2\nB1\nB2\n",
    );
}

/// A value mixing `;` and a newline: the re-read stream applies both
/// separator forms.
#[test]
fn multiline_value_mixing_semicolon_and_newline() {
    assert_clean(
        "mix",
        "shopt -s expand_aliases\n\
         alias foo1='echo one; echo two'\n\
         alias fooM='echo a; echo b\n\
         echo c'\n\
         fooM\n",
        "a\nb\nc\n",
    );
}

/// Invocation arguments glue onto the value's LAST line (the pushed
/// text ends mid-command when the value has no trailing blank).
#[test]
fn multiline_value_invocation_argument_glues_to_last_line() {
    assert_clean(
        "arg",
        "shopt -s expand_aliases\n\
         alias foo1='echo one; echo two'\n\
         alias foo2='echo three\n\
         echo four'\n\
         foo2 X\n",
        "three\nfour X\n",
    );
}

/// A value whose last line is empty (trailing newline inside the
/// quotes): expansion is one complete command, and the following
/// script line runs normally.
#[test]
fn multiline_value_trailing_newline() {
    assert_clean(
        "trail",
        "shopt -s expand_aliases\n\
         alias foo1='echo one; echo two'\n\
         alias foo4='echo five\n\
         '\n\
         foo4\n\
         echo after\n",
        "five\nafter\n",
    );
}

/// Chain cell: the expansion feeds a command substitution whose
/// captured output is printed by a later command.
#[test]
fn multiline_value_expands_inside_command_substitution() {
    assert_clean(
        "comsub",
        "shopt -s expand_aliases\n\
         alias foo1='echo one; echo two'\n\
         alias foo2='echo three\n\
         echo four'\n\
         out=$(foo2)\n\
         echo \"[$out]\"\n",
        "[three\nfour]\n",
    );
}

// ---------------------------------------------------------------------
// Control cells (green on master, must stay green)
// ---------------------------------------------------------------------

/// Plain single-line alias expansion.
#[test]
fn control_singleline_alias_expands() {
    assert_clean(
        "ctl-single",
        "shopt -s expand_aliases\n\
         alias foo1='echo one; echo two'\n\
         foo1\n",
        "one\ntwo\n",
    );
}

/// A multi-line definition read while the table is still EMPTY takes
/// the non-alias gather arm and already expands on master.
#[test]
fn control_multiline_first_alias_expands() {
    assert_clean(
        "ctl-first",
        "shopt -s expand_aliases\n\
         alias foo2='echo three\n\
         echo four'\n\
         foo2\n",
        "three\nfour\n",
    );
}

/// The same newline-carrying value delivered through $'...' on ONE
/// physical line: no multi-line definition is involved.
#[test]
fn control_ansic_newline_value_expands() {
    assert_clean(
        "ctl-ansic",
        "shopt -s expand_aliases\n\
         alias foo3=$'echo five\\necho six'\n\
         foo3\n",
        "five\nsix\n",
    );
}

/// GNU's same-read-unit rule must survive the fix: an alias defined
/// and used in the SAME read unit is not expanded (the definition is
/// not visible to tokens the reader already consumed).
#[test]
fn control_same_line_definition_not_visible() {
    let (stdout, stderr, code) = run_script(
        "ctl-sameline",
        "shopt -s expand_aliases\n\
         alias zz='echo Z'; zz\n\
         echo \"rc=$?\"\n",
    );
    assert_eq!(stdout, "rc=127\n", "ctl-sameline: stdout");
    assert!(
        stderr.contains("zz: command not found"),
        "ctl-sameline: stderr names the unexpanded command: {stderr:?}"
    );
    assert_eq!(code, Some(0), "ctl-sameline: exit code");
}

/// Registration and the query form were never broken: the stored
/// value keeps its embedded newline byte-for-byte.
#[test]
fn control_alias_query_prints_multiline_value() {
    assert_clean(
        "ctl-query",
        "shopt -s expand_aliases\n\
         alias foo1='echo one; echo two'\n\
         alias foo2='echo three\n\
         echo four'\n\
         alias foo2\n",
        "alias foo2='echo three\necho four'\n",
    );
}
