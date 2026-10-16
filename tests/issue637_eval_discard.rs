//! rubash#637: `eval` contains the assignment DISCARD jump.
//!
//! GNU catches DISCARD at the eval boundary and returns failure instead of
//! unwinding further:
//!
//! ```c
//! /* builtins/evalstring.c:365,422-442 parse_and_execute() */
//! /* Provide a location for functions which 'longjmp (top_level)' ... */
//! case DISCARD: ... last_command_exit_value = EXECUTION_FAILURE; ...
//!   /* outside a subshell: continue (return from parse_and_execute) */
//! ```
//!
//! rubash contained `ExpansionFailure` at the eval boundary but let the
//! `evalerror_pending` abort escape, so `eval 'a[-2]=q'` ended the whole
//! script. The boundary now absorbs exactly the bad-subscript class
//! (armed alongside the abort); arithmetic and nounset failures keep
//! escaping, and a real `Err` propagates untouched. Inside eval, GNU's
//! parse_and_execute loops per line: a later line still runs and eval
//! returns its status.
//!
//! Expectations come from the C source above; spot-checked against GNU Bash
//! 5.3.20 locally (the repo oracle is WSL 5.3.0 — no version-specific
//! behavior is asserted here).

use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    status: i32,
}

fn run(script: &str) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        status: output.status.code().unwrap_or(-1),
    }
}

fn run_file(script: &str) -> Run {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("issue637_{}.sh", std::process::id()));
    std::fs::write(&path, script).expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_file(&path);
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        status: output.status.code().unwrap_or(-1),
    }
}

/// A contained bad subscript in eval: one diagnostic, eval fails, caller
/// continues with status 0.
fn assert_eval_contained(script: &str, diagnostic: &str, after_stdout: &str) {
    let out = run(script);
    assert_eq!(
        out.stderr,
        format!("bash: line 1: {diagnostic}\n"),
        "{script} -> stderr {:?}",
        out.stderr
    );
    assert_eq!(
        out.stdout, after_stdout,
        "{script} -> stdout {:?}",
        out.stdout
    );
    assert_eq!(out.status, 0, "{script} -> status {:?}", out.status);
}

#[test]
fn plain_element_contained() {
    assert_eval_contained(
        "eval 'a=(p); a[-2]=q; echo IN'; echo AFTER",
        "a[-2]: bad array subscript",
        "AFTER\n",
    );
}

#[test]
fn empty_subscript_contained() {
    assert_eval_contained(
        "eval 'a=(p); a[]=v; echo IN'; echo AFTER",
        "a[]: bad array subscript",
        "AFTER\n",
    );
}

#[test]
fn nested_eval_contained() {
    assert_eval_contained(
        "eval 'eval \"a[-2]=q\"; echo NESTED'; echo AFTER",
        "a[-2]: bad array subscript",
        "NESTED\nAFTER\n",
    );
}

#[test]
fn eval_in_function_contained() {
    assert_eval_contained(
        "f(){ eval 'a[-2]=q; echo IN'; echo FUNC; }; f; echo AFTER",
        "a[-2]: bad array subscript",
        "FUNC\nAFTER\n",
    );
}

#[test]
fn eval_failure_status_observed() {
    // eval itself returns the failure: `||` and `$?` see it.
    let out = run("eval 'a[-2]=q'; echo \"rc=$?\"; echo AFTER");
    assert_eq!(out.stdout, "rc=1\nAFTER\n");
    assert_eq!(out.status, 0);
    let out = run("eval 'a[-2]=q' || echo OR-RAN; echo AFTER");
    assert_eq!(out.stdout, "OR-RAN\nAFTER\n");
    assert_eq!(out.status, 0);
}

#[test]
fn multiline_eval_continues_and_returns_last_status() {
    // Newline-separated lines inside eval are independent: a later line
    // runs and eval returns its status (GNU parse_and_execute loop).
    let out = run_file("eval $'a[-2]=q\\necho IN-NL'\necho \"rc=$?\"\necho AFTER");
    assert_eq!(out.stdout, "IN-NL\nrc=0\nAFTER\n");
    assert_eq!(out.status, 0);
}

#[test]
fn arithmetic_still_escapes() {
    // Arithmetic failures are a different class: eval never returns.
    let out = run("eval 'a+=( [1+]=q )'; echo AFTER");
    assert_eq!(out.stdout, "");
    assert_eq!(out.status, 1);
    let out = run("eval 'a+=( [1+]=q )' || echo OR-RAN; echo AFTER");
    assert_eq!(out.stdout, "");
    assert_eq!(out.status, 1);
}

#[test]
fn compound_negative_contained() {
    assert_eval_contained(
        "eval 'a=(p); a+=( [-2]=q ); echo IN'; echo AFTER",
        "[-2]=q: bad array subscript",
        "AFTER\n",
    );
}

#[test]
fn top_level_still_abandons() {
    // Outside eval: same-line rest is abandoned with status 1, but the
    // top-level DISCARD is line-bounded — the next line runs (pre-existing
    // correct behavior, unchanged by this fix).
    let out = run("a=(p); a[-2]=q; echo AFTER");
    assert_eq!(out.stdout, "");
    assert_eq!(out.status, 1);
    let out = run_file("a=(p); a[-2]=q\necho AFTER");
    assert_eq!(out.stdout, "AFTER\n");
    assert_eq!(out.status, 0);
}
