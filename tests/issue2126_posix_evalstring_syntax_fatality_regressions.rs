//! rubash#2126 — a syntax error inside `eval`/`source` in a
//! non-interactive POSIX-mode shell must exit the shell with status
//! 2; rubash reported the error, had the builtin return 2, and kept
//! executing.
//!
//! GNU: parse_and_execute's parse-failure branch
//! (builtins/evalstring.c:588-596) calls jump_to_top_level (ERREXIT)
//! with last_command_exit_value = EX_BADUSAGE (2) iff the shell is
//! non-interactive, posixly_correct, not executing under `command`
//! (executing_command_builtin == 0), and `this_shell_builtin`
//! (builtins/common.c:70) is the source or eval builtin at the moment
//! the parser reports EX_BADSYNTAX (parse.y report_syntax_error sets
//! it only while executing_builtin && parse_and_execute_level).
//!
//! this_shell_builtin is assigned on EVERY simple-command dispatch
//! (execute_cmd.c:4813; NULL for function and disk-command dispatches)
//! and never restored when a command completes — so the fatality is
//! decided by the most recent dispatch in the current flow: the
//! eval/source that opened the string arms it, any other dispatch
//! inside the string before the bad parse unit disarms it, assignment-
//! only / subshell / comsub units (no dispatch in this flow) leave it
//! armed, and a `command`/`builtin` re-dispatch re-arms it (the set is
//! unconditional; only the CHECK is gated by executing_command_
//! builtin). Forked contexts (comsub, pipeline elements) run in
//! copies and cannot move the parent's value; rubash mirrors that
//! with a ShellState bit (cloned across fork boundaries, bracketed
//! around pipeline stages).
//!
//! The jump is immediate: tested contexts (`||`, if, `!`) do not
//! contain it, the ERR trap never runs, the EXIT trap still runs.
//! Parse units are newline-terminated lists, so a bad unit after a
//! good line lets the good line run first, while `;` does not split
//! units (nothing of the unit runs).
//!
//! All expectations measured against GNU bash 5.3.20 with script
//! files (self-compiled oracle, 2026-10-30), then re-verified
//! differentially against the fixed binary.

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2126-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

/// Run `script` with the given extra fixture files in the case dir.
fn run(tag: &str, script: &str, fixtures: &[(&str, &str)]) -> (String, String, Option<i32>) {
    let dir = tempdir(tag);
    for (name, body) in fixtures {
        std::fs::write(dir.join(name), body).expect("write fixture");
    }
    let case = dir.join("case.sh");
    std::fs::write(&case, script).expect("write case");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg(&case)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &dir)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

const BADONLY: (&str, &str) = ("badonly.sh", "if true\n");

fn posix(script: &str) -> String {
    format!("set -o posix\n{script}\n")
}

/// POSIX fatal face: diagnostic naming the syntax error, exit 2, and
/// none of the post-error markers run.
fn assert_posix_syntax_exits(tag: &str, script: &str, fixtures: &[(&str, &str)]) {
    let (stdout, stderr, code) = run(tag, &posix(script), fixtures);
    assert!(
        !stdout.contains("AFTER"),
        "{tag}: stdout must not reach AFTER: {stdout}"
    );
    assert!(
        stderr.contains("syntax error"),
        "{tag}: stderr must carry the syntax diagnostic: {stderr}"
    );
    assert_eq!(code, Some(2), "{tag}: exit code (stderr: {stderr})");
}

/// Continuing face: the builtin returns 2, the script runs on.
fn assert_syntax_continues(tag: &str, script: &str, fixtures: &[(&str, &str)]) -> String {
    let (stdout, stderr, code) = run(tag, &posix(script), fixtures);
    assert!(
        stdout.contains("AFTER"),
        "{tag}: stdout must reach AFTER: {stdout}"
    );
    assert_eq!(code, Some(0), "{tag}: exit code (stderr: {stderr})");
    stdout
}

// ---- POSIX fatal: bare eval ----------------------------------------------

#[test]
fn issue2126_posix_eval_syntax_error_exits() {
    assert_posix_syntax_exits("eval", "eval 'if true'\necho AFTER", &[]);
}

#[test]
fn issue2126_posix_eval_syntax_error_not_contained() {
    for (tag, body) in [
        ("eval-or", "eval 'if true' || echo OR\necho AFTER"),
        (
            "eval-if",
            "if eval 'if true'; then echo THEN; fi\necho AFTER",
        ),
        ("eval-not", "! eval 'if true'\necho AFTER"),
        ("eval-and", "eval 'if true' && echo AND\necho AFTER"),
    ] {
        assert_posix_syntax_exits(tag, body, &[]);
    }
}

#[test]
fn issue2126_posix_eval_syntax_error_skips_err_trap_but_runs_exit_trap() {
    let (stdout, stderr, code) = run(
        "eval-traps",
        &posix("trap 'echo ERRTRAP' ERR\ntrap 'echo EXITT' EXIT\neval 'if true'\necho AFTER"),
        &[],
    );
    assert!(!stdout.contains("ERRTRAP"), "no ERR trap: {stdout}");
    assert!(stdout.contains("EXITT"), "EXIT trap runs: {stdout}");
    assert!(!stdout.contains("AFTER"), "no AFTER: {stdout}");
    assert_eq!(code, Some(2), "exit code (stderr: {stderr})");
}

#[test]
fn issue2126_posix_eval_semicolon_is_one_parse_unit() {
    // `;` does not split parse units: nothing of the unit runs.
    let (stdout, stderr, code) = run(
        "eval-semi",
        &posix("eval 'echo FIRSTLINE; if true'\necho AFTER"),
        &[],
    );
    assert!(!stdout.contains("FIRSTLINE"), "no FIRSTLINE: {stdout}");
    assert!(!stdout.contains("AFTER"), "no AFTER: {stdout}");
    assert_eq!(code, Some(2), "exit code (stderr: {stderr})");
}

#[test]
fn issue2126_posix_eval_bad_function_body_exits() {
    assert_posix_syntax_exits("eval-funcbody", "eval 'f(){ if true;}'\necho AFTER", &[]);
}

#[test]
fn issue2126_posix_builtin_eval_syntax_error_exits() {
    assert_posix_syntax_exits("builtin-eval", "builtin eval 'if true'\necho AFTER", &[]);
}

// ---- POSIX fatal: source ---------------------------------------------------

#[test]
fn issue2126_posix_source_syntax_error_exits() {
    assert_posix_syntax_exits("source", "source ./badonly.sh\necho AFTER", &[BADONLY]);
}

#[test]
fn issue2126_posix_dot_syntax_error_exits() {
    assert_posix_syntax_exits("dot", ". ./badonly.sh\necho AFTER", &[BADONLY]);
}

#[test]
fn issue2126_posix_nested_source_syntax_error_exits() {
    assert_posix_syntax_exits(
        "nested",
        "source ./outer.sh\necho AFTER",
        &[BADONLY, ("outer.sh", "source ./badonly.sh\n")],
    );
}

#[test]
fn issue2126_posix_source_inside_function_syntax_error_exits() {
    assert_posix_syntax_exits(
        "source-in-func",
        "f(){ source ./badonly.sh; }\nf\necho AFTER",
        &[BADONLY],
    );
}

#[test]
fn issue2126_posix_source_rearm_through_eval_exits() {
    // The eval'd string dispatches `source` (a re-arm), whose file's
    // first unit is bad: fatal, with the good prefix printed.
    let (stdout, stderr, code) = run(
        "rearm",
        &posix("eval 'echo A\nsource ./badonly.sh'\necho AFTER"),
        &[BADONLY],
    );
    assert!(stdout.contains('A'), "prefix runs: {stdout}");
    assert!(!stdout.contains("AFTER"), "no AFTER: {stdout}");
    assert_eq!(code, Some(2), "exit code (stderr: {stderr})");
}

// ---- POSIX fatal: the arming set (no dispatch before the bad unit) ---------

#[test]
fn issue2126_posix_assignment_first_in_file_stays_armed() {
    assert_posix_syntax_exits(
        "assign-first",
        "source ./f.sh\necho AFTER",
        &[("f.sh", "x=1\nif true\n")],
    );
}

#[test]
fn issue2126_posix_subshell_first_in_file_stays_armed() {
    // A subshell unit dispatches in a forked copy; the parent's
    // tracker is untouched, so the bad unit after it is still fatal —
    // but the subshell's own output already happened.
    let (stdout, stderr, code) = run(
        "subshell-first",
        &posix("source ./f.sh\necho AFTER"),
        &[("f.sh", "(echo SUBA)\nif true\n")],
    );
    assert!(stdout.contains("SUBA"), "subshell prefix runs: {stdout}");
    assert!(!stdout.contains("AFTER"), "no AFTER: {stdout}");
    assert_eq!(code, Some(2), "exit code (stderr: {stderr})");
}

#[test]
fn issue2126_posix_comsub_assignment_first_in_file_stays_armed() {
    assert_posix_syntax_exits(
        "comsub-first",
        "source ./f.sh\necho AFTER",
        &[("f.sh", "x=$(echo A)\nif true\n")],
    );
}

#[test]
fn issue2126_posix_completed_eval_without_dispatch_stays_armed() {
    // `eval 'x=1'` dispatches nothing inside its string, so the
    // tracker is still the eval builtin when the bad unit parses.
    assert_posix_syntax_exits(
        "eval-nodispatch",
        "source ./f.sh\necho AFTER",
        &[("f.sh", "eval 'x=1'\nif true\n")],
    );
}

#[test]
fn issue2126_posix_command_eval_redispatch_rearms() {
    // The re-dispatch SETS this_shell_builtin even though the CHECK
    // is gated while that eval runs.
    assert_posix_syntax_exits(
        "command-rearm",
        "source ./f.sh\necho AFTER",
        &[("f.sh", "command eval 'x=1'\nif true\n")],
    );
}

#[test]
fn issue2126_posix_builtin_eval_redispatch_rearms() {
    assert_posix_syntax_exits(
        "builtin-rearm",
        "source ./f.sh\necho AFTER",
        &[("f.sh", "builtin eval 'x=1'\nif true\n")],
    );
}

// ---- POSIX continuing: a dispatch inside the string disarms -----------------

#[test]
fn issue2126_posix_command_eval_syntax_error_continues() {
    let stdout = assert_syntax_continues(
        "command-eval",
        "command eval 'if true'\necho \"AFTER rc=$?\"",
        &[],
    );
    // Contained does not mean free: eval still returns EX_BADUSAGE.
    assert!(stdout.contains("AFTER rc=2"), "stdout: {stdout}");
}

#[test]
fn issue2126_posix_command_source_syntax_error_continues() {
    assert_syntax_continues(
        "command-source",
        "command source ./badonly.sh\necho AFTER",
        &[BADONLY],
    );
}

#[test]
fn issue2126_posix_good_line_before_bad_unit_in_file_continues() {
    let stdout = assert_syntax_continues(
        "badmid",
        "source ./f.sh\necho AFTER",
        &[("f.sh", "echo FIRSTLINE\nif true\n")],
    );
    assert!(stdout.contains("FIRSTLINE"), "prefix runs: {stdout}");
}

#[test]
fn issue2126_posix_eval_newline_separated_units_continue() {
    let stdout = assert_syntax_continues(
        "eval-newline",
        "eval 'echo FIRSTLINE\nif true'\necho AFTER",
        &[],
    );
    assert!(stdout.contains("FIRSTLINE"), "prefix runs: {stdout}");
}

#[test]
fn issue2126_posix_dispatch_inside_string_disarms() {
    for (tag, fixture) in [
        ("blt-first", "true\nif true\n"),
        ("ext-first", "/bin/true\nif true\n"),
        ("func-first", "f(){ x=1; }\nf\nif true\n"),
        ("eval-true-first", "eval 'true'\nif true\n"),
    ] {
        assert_syntax_continues(tag, "source ./f.sh\necho AFTER", &[("f.sh", fixture)]);
    }
}

#[test]
fn issue2126_posix_eval_dispatch_inside_eval_string_disarms() {
    let stdout =
        assert_syntax_continues("eval-echo-first", "eval 'echo A\nif true'\necho AFTER", &[]);
    assert!(stdout.contains('A'), "prefix runs: {stdout}");
}

#[test]
fn issue2126_posix_syntax_error_in_comsub_is_contained() {
    assert_syntax_continues("comsub", "x=$(eval 'if true')\necho AFTER", &[]);
}

#[test]
fn issue2126_posix_syntax_error_in_pipeline_element_is_contained() {
    assert_syntax_continues("pipeline", "eval 'if true' | cat\necho AFTER", &[]);
}

// ---- Non-POSIX controls + ordinary eval behavior -----------------------------

#[test]
fn issue2126_nonposix_eval_and_source_syntax_errors_continue() {
    let (stdout, stderr, code) = run(
        "nonposix",
        "eval 'if true'\nsource ./badonly.sh\necho AFTER",
        &[BADONLY],
    );
    assert!(stdout.contains("AFTER"), "stdout: {stdout}");
    assert_eq!(code, Some(0), "exit code (stderr: {stderr})");
}

#[test]
fn issue2126_posix_eval_runtime_status_unchanged() {
    let (stdout, stderr, code) = run(
        "runtime",
        &posix("eval 'false'\necho \"AFTER rc=$?\"\neval ''\necho \"EMPTY rc=$?\""),
        &[],
    );
    assert!(stdout.contains("AFTER rc=1"), "stdout: {stdout}");
    assert!(stdout.contains("EMPTY rc=0"), "stdout: {stdout}");
    assert_eq!(code, Some(0), "exit code (stderr: {stderr})");
}
