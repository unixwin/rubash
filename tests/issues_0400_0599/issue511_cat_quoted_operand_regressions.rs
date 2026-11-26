//! issue#511 regression: the in-engine `external_cat` file-operand loop ran
//! each operand through `expand_word` a SECOND time. The simple-command
//! executor had already expanded the argv once (execute_cmd.c
//! execute_disk_command -> expand_words), so a single-quoted operand like
//! `'$FIX/c'` arrived with its quotes removed and a live-looking `$` — and
//! the second pass re-scanned it, silently opening `zz/c` where GNU cat
//! reports `cat: $FIX/c: No such file or directory` and exits 1. Silent
//! wrong-file reads are data corruption, hence the P0.
//!
//! The fix uses the argv as-is in the operand loop, the same rule `external_sed`
//! adopted in rubash#177. `parse_cat_argv` classifies options from the same
//! final argv (getopt sees the post-expansion text, exactly like GNU cat.exe).
//!
//! Bisect note (2026-10-12): every commit of 41de8319..origin/master
//! reproduces the bug, back through 41de8319 to the initial import 896ef0d4
//! ("chore: init") — the second expansion predates the #501 hot-path rework,
//! which the issue suspected. No introducing commit exists in history; the
//! fix is behavioral, not a revert.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn shell_bin() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_rubash") {
        return PathBuf::from(path);
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(profile)
        .join(if cfg!(windows) {
            "rubash.exe"
        } else {
            "rubash"
        })
}

struct Output {
    status: i32,
    stdout: String,
    stderr: String,
}

/// Run `script` via -c inside a fresh scratch dir that already contains
/// `zz/c` holding `hi` (the issue#511 fixture) and return the three channels.
fn run_in_fixture(script: &str, tag: &str) -> Output {
    let dir = std::env::temp_dir().join(format!("rubash-issue511-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("zz")).expect("scratch dir");
    std::fs::write(dir.join("zz/c"), b"hi").expect("fixture zz/c");

    let mut child = Command::new(shell_bin())
        .args(["-c", script])
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    // Feed stdin closed-but-nonblocking: none of these scripts read stdin,
    // and Stdio::null() above keeps `cat -`-less runs from blocking.
    let _ = child.stdin.take();
    let output = child.wait_with_output().expect("wait for rubash");
    let _ = std::fs::remove_dir_all(&dir);
    Output {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).replace('\r', ""),
    }
}

/// The exact repro from the issue: a single-quoted operand must reach cat
/// literally — GNU errors, it never reads zz/c.
#[test]
fn single_quoted_variable_operand_is_not_re_expanded() {
    let out = run_in_fixture("FIX=zz; cat '$FIX/c'", "single");
    assert!(
        !out.stdout.contains("hi"),
        "cat silently read zz/c for quoted '$FIX/c': stdout={:?} stderr={:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stderr.contains("$FIX/c") && out.stderr.contains("No such file or directory"),
        "expected the literal-operand GNU diagnostic, got stderr={:?}",
        out.stderr
    );
    assert_eq!(out.status, 1, "cat on a missing operand exits 1");
}

/// The escaped form `\$FIX/c` is the same literal text after the first
/// expansion pass — it must not re-expand either.
#[test]
fn escaped_dollar_operand_is_literal() {
    let out = run_in_fixture("FIX=zz; cat \\$FIX/c", "escaped");
    assert!(
        !out.stdout.contains("hi"),
        "cat silently read zz/c for \\$FIX/c: stdout={:?} stderr={:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stderr.contains("$FIX/c") && out.stderr.contains("No such file or directory"),
        "stderr={:?}",
        out.stderr
    );
    assert_eq!(out.status, 1);
}

/// An unset variable in single quotes: the diagnostic must carry the literal
/// operand text, not the empty expansion of a second pass.
#[test]
fn unset_variable_operand_error_keeps_literal_name() {
    let out = run_in_fixture("cat '$NOSUCHVAR511'", "unset");
    assert!(
        out.stderr.contains("$NOSUCHVAR511"),
        "error text lost the literal operand: stderr={:?}",
        out.stderr
    );
    assert_eq!(out.status, 1);
}

/// Quote fidelity must not swallow live expansions: the double-quoted and
/// unquoted forms still read the fixture (GNU parity both ways).
#[test]
fn double_quoted_and_unquoted_operands_still_expand() {
    let out = run_in_fixture("FIX=zz; cat \"$FIX/c\"", "dq");
    assert_eq!(out.stdout, "hi", "double-quoted operand must expand");
    assert_eq!(out.status, 0);

    let out = run_in_fixture("FIX=zz; cat $FIX/c", "uq");
    assert_eq!(out.stdout, "hi", "unquoted operand must expand");
    assert_eq!(out.status, 0);
}

/// The rubash#177 hazard, cat edition: quote-protected `$(...)` and backtick
/// text in an operand is DATA. The second expansion executed it.
#[test]
fn command_substitution_text_in_quotes_is_data_not_code() {
    let out = run_in_fixture("cat 'zz/$(echo pwned)'", "comsub");
    assert!(
        !out.stdout.contains("pwned"),
        "$(echo pwned) in a single-quoted operand was EXECUTED: stdout={:?}",
        out.stdout
    );
    assert!(
        out.stderr.contains("$(echo pwned)"),
        "literal operand text missing from diagnostic: stderr={:?}",
        out.stderr
    );
    assert_eq!(out.status, 1);

    let out = run_in_fixture("cat 'zz/`echo pwned`'", "backtick");
    assert!(
        !out.stdout.contains("pwned"),
        "backticks in a single-quoted operand were EXECUTED: stdout={:?}",
        out.stdout
    );
    assert_eq!(out.status, 1);
}

/// GNU cat.c: operand failures print a diagnostic and set exit 1 but do not
/// stop the loop — a later literal operand is still read.
#[test]
fn failing_quoted_operand_does_not_stop_later_operands() {
    let out = run_in_fixture("FIX=zz; cat '$FIX/c' zz/c", "mixed");
    assert!(out.stderr.contains("$FIX/c"), "stderr={:?}", out.stderr);
    assert!(
        out.stdout.contains("hi"),
        "cat stopped at the failed operand; GNU continues: stdout={:?}",
        out.stdout
    );
    assert_eq!(out.status, 1, "any operand failure leaves status 1");
}

/// The inline pipeline stage and the command-substitution cat share the
/// operand rule: single-quoted text stays literal there too.
#[test]
fn pipeline_stage_cat_honors_single_quotes() {
    let out = run_in_fixture("FIX=zz; printf 'pipe\\n' | cat '$FIX/c'", "pipeline");
    assert!(
        !out.stdout.contains("hi"),
        "pipeline cat re-expanded the quoted operand: stdout={:?} stderr={:?}",
        out.stdout,
        out.stderr
    );
    assert!(out.stderr.contains("$FIX/c"), "stderr={:?}", out.stderr);
}

#[test]
fn command_substitution_cat_honors_single_quotes() {
    let out = run_in_fixture("FIX=zz; out=$(cat '$FIX/c'); echo \"[$out]\"", "comsub-cat");
    assert_eq!(
        out.stdout, "[]\n",
        "$(...) cat re-expanded the quoted operand: stdout={:?} stderr={:?}",
        out.stdout, out.stderr
    );
    assert!(out.stderr.contains("$FIX/c"), "stderr={:?}", out.stderr);
}

/// Guard the neighboring contract while touching this loop: the `-` operand
/// still means stdin at its position.
#[test]
fn dash_operand_still_reads_stdin() {
    let dir = std::env::temp_dir().join(format!("rubash-issue511-dash-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");

    let mut child = Command::new(shell_bin())
        .args(["-c", "cat -"])
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    {
        let mut stdin = child.stdin.take().expect("stdin pipe");
        let _ = stdin.write_all(b"dashdata\n");
    }
    let output = child.wait_with_output().expect("wait for rubash");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "dashdata\n");
    assert_eq!(output.status.code(), Some(0));
}
