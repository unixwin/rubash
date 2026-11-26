//! Issue rubash#522 — syntax-error message token echo fidelity (cosmetic;
//! exit statuses already matched).
//!
//! GNU's parser error report (error.c:300-316 parser_error +
//! report_syntax_error) carries:
//! - the `line N:` segment (the offending line, error.c:300-316), and
//! - the offending text VERBATIM — `for ((i=0; i<3)); do :; done` echoes
//!   `((i=0; i<3))` with the source's own spacing (parse.y:6813-6826
//!   print_offending_line echoes shell_input_line as read; the arith-for
//!   error path echoes the between-parens text unnormalized).
//! Rubash normalized the echo (`(( i=0; i<3 ))` — synthesized `(( {} ))`
//! wrapper + trimmed inner text) and dropped the line segment (the error
//! node had no line, so the prefix fell back to the bare `bash: ` form).
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (script-file probes, 2026-10-12; the program-name segment is
//! normalization-normalized per the issue and asserted only structurally).

use std::process::Command;

/// Run a SCRIPT FILE (the byte-exact baseline shape) and return stderr.
fn rubash_file(script: &str) -> String {
    let path = std::env::temp_dir().join(format!(
        "rubash_i522_{}_{}.sh",
        std::process::id(),
        script.len()
    ));
    std::fs::write(&path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_file(&path);
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The arith-for header error carries the line segment and the verbatim
/// header text (GNU: `line 1: syntax error: arithmetic expression
/// required` then ``line 1: syntax error: `((i=0; i<3))'``).
#[test]
fn arith_for_header_error_is_verbatim_with_line() {
    let stderr = rubash_file("for ((i=0; i<3)); do :; done\n");
    assert!(
        stderr.contains("line 1: syntax error: arithmetic expression required\n"),
        "stderr={stderr:?}"
    );
    assert!(
        stderr.contains("line 1: syntax error: `((i=0; i<3))'\n"),
        "stderr={stderr:?}"
    );
}

/// The echo preserves the source's own spacing (extra blanks are NOT
/// normalized: GNU echoes `((  i=0;  i<3  ))` for the doubled-space
/// header).
#[test]
fn arith_for_header_echo_preserves_source_spacing() {
    let stderr = rubash_file("for ((  i=0;  i<3  )); do :; done\n");
    assert!(
        stderr.contains("line 1: syntax error: `((  i=0;  i<3  ))'\n"),
        "stderr={stderr:?}"
    );
}

/// Guard: the error report keeps the two-line shape (parser_error + the
/// echoed text) whatever the line number — a line-2 header reports line 2.
#[test]
fn arith_for_error_reports_its_own_line() {
    let stderr = rubash_file("echo first\nfor ((i=0; i<3)); do :; done\n");
    assert!(
        stderr.contains("line 2: syntax error: arithmetic expression required\n"),
        "stderr={stderr:?}"
    );
    assert!(
        stderr.contains("line 2: syntax error: `((i=0; i<3))'\n"),
        "stderr={stderr:?}"
    );
}

/// Exit status stays 2 (GNU: rc=2, already matched before the fix).
#[test]
fn arith_for_error_exit_status_is_two() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("for ((i=0; i<3)); do :; done")
        .output()
        .expect("run rubash");
    assert_eq!(output.status.code(), Some(2));
}
