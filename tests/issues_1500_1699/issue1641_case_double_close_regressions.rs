//! rubash#1641 — a `case` clause whose pattern list is closed twice
//! (`x))`, `(x))`, `x|y))`) was accepted and the clause body executed.
//!
//! GNU grammar (parse.y:1227-1236 pattern_list): after the pattern's
//! closing `)` only a compound_list or a newline_list (bodyless
//! clause) may follow; compound_list (parse.y:1262) is
//! `newline_list list0` and no list can begin with `)`. The second
//! `)` is therefore a parse-time syntax error: the clause never
//! executes and the shell exits 2.
//!
//! Expectations hand-measured against GNU bash 5.3.20 (script files
//! and `-c` probes): stderr is
//! `syntax error near unexpected token `)'` with the offending line
//! echoed, stdout stays empty, rc is 2.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir =
        std::env::temp_dir().join(format!("rubash-issue1641-{}-{tag}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn run_c(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn run_script(script: &str) -> (String, String, Option<i32>) {
    let dir = scratch("script");
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(path.to_str().expect("utf8 path"))
        .output()
        .expect("run rubash");
    let out = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    out
}

fn assert_stray_paren_rejected(stdout: &str, stderr: &str, code: Option<i32>) {
    assert_eq!(code, Some(2), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(stdout, "", "clause body must not execute: {stdout}");
    assert!(
        stderr.contains("syntax error near unexpected token `)'"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1641_double_close_after_bare_pattern_is_syntax_error() {
    let (stdout, stderr, code) = run_script("case x in x)) echo A;; esac\n");
    assert_stray_paren_rejected(&stdout, &stderr, code);
}

#[test]
fn issue1641_double_close_after_paren_pattern_is_syntax_error() {
    let (stdout, stderr, code) = run_script("case x in (x)) echo A;; esac\n");
    assert_stray_paren_rejected(&stdout, &stderr, code);
}

#[test]
fn issue1641_double_close_after_alternation_is_syntax_error() {
    let (stdout, stderr, code) = run_script("case x in x|y)) echo A;; esac\n");
    assert_stray_paren_rejected(&stdout, &stderr, code);
}

#[test]
fn issue1641_double_close_in_later_clause_is_syntax_error() {
    // The error is parse-time: even the well-formed first clause must
    // not execute (GNU reports the error before running anything).
    let (stdout, stderr, code) = run_script("case x in x) echo A;; y)) echo B;; esac\n");
    assert_stray_paren_rejected(&stdout, &stderr, code);
}

#[test]
fn issue1641_double_close_in_dash_c_is_syntax_error() {
    let (stdout, stderr, code) = run_c("case x in x)) echo A;; esac");
    assert_stray_paren_rejected(&stdout, &stderr, code);
}

#[test]
fn issue1641_stray_paren_on_line_after_close_is_syntax_error() {
    // GNU reads the newline_list after `)` and still rejects a `)`
    // that then tries to open the body, citing that token's line.
    let (stdout, stderr, code) = run_script("case x in\nx)\n)\necho A;;\nesac\n");
    assert_stray_paren_rejected(&stdout, &stderr, code);
    assert!(stderr.contains("line 3"), "stderr: {stderr}");
}

#[test]
fn issue1641_single_close_forms_still_execute() {
    for script in [
        "case x in x) echo A;; esac\n",
        "case x in (x) echo A;; esac\n",
        "case x in x|y) echo A;; esac\n",
        "case x in x)\necho A;;\nesac\n",
        "case x in x) ;; esac\necho done\n",
        "case x in\nx)\nesac\necho done\n",
    ] {
        let (stdout, stderr, code) = run_script(script);
        assert_eq!(code, Some(0), "script: {script} stderr: {stderr}");
        assert!(
            stdout == "A\n" || stdout == "done\n",
            "script: {script} stdout: {stdout}"
        );
    }
}

#[test]
fn issue1641_extglob_close_then_clause_close_still_executes() {
    // `@(x|y)` carries its own `)`; the token after it is the
    // clause's closing `)`, not a stray.
    let (stdout, stderr, code) = run_script("shopt -s extglob\ncase x in @(x|y)) echo A;; esac\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "A\n", "stderr: {stderr}");
}
