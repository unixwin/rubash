//! Issue unixwin/rubash#3205: a same-line `shopt -s extglob` leaks
//! into the lexing of the SAME read unit's later segments.
//!
//! GNU contract: a read unit (one complete_command — a whole
//! `;`/`&&`/`||`/pipeline list, however many physical lines it
//! spans) is read in full before any of it executes
//! (parse.y read_command; the and-or grammar productions at
//! parse.y:1286-1288 read the whole list before
//! execute_connection runs any operand). `read_token_word`
//! consults the read-time `extended_glob` flag per token
//! (parse.y:5466), so the option state at the unit's START decides
//! the lexing of every word in the unit; a `shopt` inside the unit
//! takes effect only from the NEXT unit. Expectations below are
//! byte-verified against GNU bash 5.3.20 (script files run with
//! cwd = a fixture directory holding exactly
//! `ab abcx abcz ba bbc cab`).
//!
//! Defect cells (red before the fix):
//! - x4: the `&&` segment was lexed with the just-executed shopt's
//!   state and glob-expanded (rc=0); GNU reads the whole list with
//!   the gate off and rejects the pattern (rc=2, no stdout — the
//!   next line never runs because a read error is fatal to the
//!   script).
//! - x1b: at command position the glued `!(` was read as an
//!   extglob pattern head and rejected (rc=2); GNU reads `!` as
//!   negation plus a `(bbc)` subshell (rc=0, `bbc: command not
//!   found`, the next line runs).
//!
//! Extended cells (same unit-entry invariant, also red before the
//! fix; pinned here so the fix is judged on the invariant, not on
//! the two reported shapes):
//! - z1: the gate was already on from a PREVIOUS unit — a same-line
//!   `shopt -s` in this unit must not turn a legitimate pattern
//!   into an error.
//! - m2: the unit spans several physical lines (an `if` body); a
//!   shopt on the unit's second line must not lex its third.
//! - y3 / v7: the shopt sits in an earlier `&&` segment; GNU still
//!   reads the whole list under the unit-start gate.
//! - w9: the pattern command is a pipeline stage after a `;`.
//!
//! The `;` argument-position cell pins rc + the first diagnostic
//! line only: GNU also echoes the offending source line, and that
//! echo's absence in grouped-mode diagnostics is #3184's face, not
//! this issue's.

use std::path::PathBuf;
use std::process::Command;

fn fixture(tag: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rubash3205-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let files = dir.join("files");
    std::fs::create_dir_all(&files).expect("create fixture dir");
    for name in ["ab", "abcx", "abcz", "ba", "bbc", "cab"] {
        std::fs::write(files.join(name), "").expect("write fixture file");
    }
    (dir, files)
}

fn run_script(tag: &str, script: &str) -> (String, String, Option<i32>, PathBuf) {
    let (dir, files) = fixture(tag);
    let file = dir.join("case.sh");
    std::fs::write(&file, script).expect("write script");
    let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&file)
        .current_dir(&files)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
        file,
    )
}

fn assert_read_error(tag: &str, script: &str) {
    let (stdout, stderr, code, _) = run_script(tag, script);
    assert_eq!(stdout, "", "{tag}: a read error leaves no stdout");
    assert!(
        stderr.contains("syntax error near unexpected token `('"),
        "{tag}: first diagnostic line names `(': {stderr:?}"
    );
    assert_eq!(code, Some(2), "{tag}: GNU exits 2 on a read error");
}

// ---------------------------------------------------------------------
// Defect cells (the brief's set)
// ---------------------------------------------------------------------

#[test]
fn issue3205_x4_and_or_segment_uses_unit_start_gate() {
    assert_read_error("x4", "shopt -s extglob && echo !(bbc)\necho after\n");
}

#[test]
fn issue3205_x1b_command_position_bang_paren_is_negated_subshell() {
    let (stdout, stderr, code, file) = run_script("x1b", "shopt -s extglob;!(bbc)\necho after\n");
    assert_eq!(stdout, "after\n");
    assert_eq!(
        stderr,
        format!("{}: line 1: bbc: command not found\n", file.display())
    );
    assert_eq!(code, Some(0));
}

// ---------------------------------------------------------------------
// Control cells (green before and after the fix)
// ---------------------------------------------------------------------

#[test]
fn issue3205_x0b_bang_paren_without_shopt() {
    let (stdout, stderr, code, file) = run_script("x0b", "!(bbc)\necho after\n");
    assert_eq!(stdout, "after\n");
    assert_eq!(
        stderr,
        format!("{}: line 1: bbc: command not found\n", file.display())
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue3205_previous_line_shopt_enables_glob() {
    let (stdout, stderr, code, _) = run_script("prevline", "shopt -s extglob\necho !(bbc)\n");
    assert_eq!(stdout, "ab abcx abcz ba cab\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3205_dash_c_newline_separated_shopt_enables_glob() {
    let (_dir, files) = fixture("dashc");
    let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("shopt -s extglob\necho !(bbc)")
        .current_dir(&files)
        .output()
        .expect("run rubash -c");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "ab abcx abcz ba cab\n"
    );
    assert_eq!(String::from_utf8_lossy(&out.stderr), "");
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn issue3205_semicolon_argument_position_rejected_by_both() {
    // GNU and rubash agree here (rc=2 naming `('); only rc and the
    // naming line are pinned — the offending-line echo is #3184.
    assert_read_error("semi", "shopt -s extglob; echo !(bbc)\necho after\n");
}

// ---------------------------------------------------------------------
// Extended cells (same invariant; red before the fix)
// ---------------------------------------------------------------------

#[test]
fn issue3205_extended_gate_already_on_from_previous_unit() {
    let (stdout, stderr, code, _) =
        run_script("z1", "shopt -s extglob\nshopt -s extglob; echo @(bbc)\n");
    assert_eq!(stdout, "bbc\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3205_extended_multiline_unit_if_body() {
    assert_read_error(
        "m2",
        "if true; then\nshopt -s extglob\necho @(bbc)\nfi\necho after\n",
    );
}

#[test]
fn issue3205_extended_shopt_in_earlier_and_or_segment() {
    assert_read_error(
        "y3",
        "true && shopt -s extglob && echo !(bbc)\necho after\n",
    );
}

#[test]
fn issue3205_extended_and_or_at_pattern() {
    assert_read_error("v7", "shopt -s extglob && echo @(bbc)\necho after\n");
}

#[test]
fn issue3205_extended_pipeline_stage_after_semicolon() {
    assert_read_error("w9", "shopt -s extglob; echo !(bbc) | cat\necho after\n");
}
