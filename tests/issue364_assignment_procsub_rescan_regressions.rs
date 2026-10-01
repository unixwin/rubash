//! Issue rubash#364 — assignment RHS must not re-execute process
//! substitutions that arrive from PARAMETER-EXPANSION RESULTS.
//!
//! Root cause (fixed with this file): the post-expansion scan in
//! `expand_assignment_value_inner` (src/executor/assignment_expansion.rs)
//! materialized `<(`/`>(` spans found in the EXPANDED assignment value, so
//! text that a variable's value carried was executed as a process
//! substitution and replaced by `/dev/fd/N`:
//!
//! ```sh
//! other='comm <(bar)'
//! line=$other            # executed `bar` and stored `comm /dev/fd/63`
//! line="${other//X/Y}"   # same — this is bats-preprocess's CR-strip line
//! ```
//!
//! GNU spec: `expand_word_internal` cases '<'/'>' (subst.c:11358-11381)
//! recognize a process substitution ONLY while the word walker scans the
//! ORIGINAL word text; results of parameter expansion are appended as data
//! via `add_string`/`sub_append_string` (subst.c:11352-11356) and are never
//! rescanned for `<('. A literal unquoted `var=<(cmd)` DOES substitute (same
//! subst.c branch — assignment words never set W_NOPROCSUB), which rubash
//! implements through the raw-RHS path
//! (`materialize_assignment_process_substitutions_from_raw`).
//!
//! This bug corrupted bats-core's `bats-preprocess` output (its
//! `line="${line//$'\r'/}"` loop executed `<(normalize_variable_list ...)`
//! inside test/bats.bats text and rewrote the preprocessed source), hanging
//! the 301-test bats.bats/bats_pipe.bats harvest (#364).
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (`/usr/local/bin/bash`, script-file runs, 2026-10-01).

use std::process::Command;

fn rubash(arg: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(arg)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// `var=$other` keeps procsub text from the variable's value literal
/// (GNU A: `line1=$other` -> `comm <(bar)`).
#[test]
fn plain_assignment_keeps_variable_borne_procsub_text() {
    let (stdout, stderr, code) =
        rubash(r#"other='comm <(bar)'; line=$other; printf '[%s]' "$line""#);
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "[comm <(bar)]");
}

/// `var="pre$other post"` — mixed literal and expansion result stays
/// literal (GNU C).
#[test]
fn mixed_assignment_keeps_variable_borne_procsub_text() {
    let (stdout, stderr, code) =
        rubash(r#"other='comm <(bar)'; line="pre $other post"; printf '[%s]' "$line""#);
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "[pre comm <(bar) post]");
}

/// `var="${other//X/Y}"` — pattern-substitution results are data (GNU D).
/// This is the exact bats-preprocess CR-strip shape from #364.
#[test]
fn patsub_assignment_keeps_variable_borne_procsub_text() {
    let (stdout, stderr, code) = rubash(
        r#"other='comm <(bar <f) <(bar <<<x)'; line="${other//X/Y}"; printf '[%s]' "$line""#,
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "[comm <(bar <f) <(bar <<<x)]");
}

/// Prefix-strip `${other#X}` results are data too (GNU: same add_string
/// path — only the word walker's own unquoted `<(` substitutes).
#[test]
fn prefix_strip_assignment_keeps_variable_borne_procsub_text() {
    let (stdout, stderr, code) =
        rubash(r#"other='comm <(bar)'; line="${other#X}"; printf '[%s]' "$line""#);
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "[comm <(bar)]");
}

/// A wholly double-quoted RHS keeps its own `<(` as data (GNU:
/// subst.c:11364 checks Q_DOUBLE_QUOTES and takes add_character).
#[test]
fn quoted_assignment_rhs_keeps_procsub_text() {
    let (stdout, stderr, code) = rubash(r#"line2="<(literal)"; printf '[%s]' "$line2""#);
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "[<(literal)]");
}

/// A LITERAL unquoted `var=<(cmd)` still substitutes (GNU B:
/// `line2=<(echo hi)` stores `/dev/fd/63`).
#[test]
fn literal_assignment_procsub_still_substitutes() {
    let (stdout, stderr, code) = rubash(r#"line2=<(printf hi); cat "$line2"; echo "rc:$?""#);
    assert!(!stderr.contains("command not found"), "stderr: {stderr}");
    assert!(stdout.contains("hi"), "stdout: {stdout}");
    assert!(stdout.contains("rc:0"), "stdout: {stdout}");
    assert!(code.is_some_and(|c| c != 2));
}

/// Command-argument position was never affected (control, GNU E).
#[test]
fn command_argument_patsub_keeps_procsub_text() {
    let (stdout, stderr, code) = rubash(r#"other='comm <(bar)'; printf '[%s]' "${other//X/Y}""#);
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "[comm <(bar)]");
}

/// The #364 reproducer shape: reading a file whose TEXT contains `<(cmd)`
/// through a CR-strip loop must not execute the file's content.
#[test]
fn cr_strip_loop_never_executes_procsub_text_from_data() {
    let dir = std::env::temp_dir().join("rubash-issue364-procsub");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let data = dir.join("data.bats");
    std::fs::write(
        &data,
        "foo() {\n  ! comm <(ghost_cmd <f) <(ghost_cmd <<<x) | grep v\n}\nfoo\n",
    )
    .unwrap();
    let script = format!(
        "while IFS= read -r line; do line=\"${{line//$'\\r'/}}\"; printf '%s\\n' \"$line\"; done < {}",
        data.to_string_lossy().replace('\\', "/")
    );
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(&script)
        .output()
        .expect("run rubash");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(!stderr.contains("ghost_cmd"), "stderr: {stderr}");
    assert!(
        stdout.contains("! comm <(ghost_cmd <f) <(ghost_cmd <<<x) | grep v"),
        "stdout: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
