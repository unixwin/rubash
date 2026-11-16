//! Issues rubash#3540 + rubash#3541 — the parameter-error pre-scan must
//! treat a command substitution inside an indexed subscript the way GNU
//! does: the comsub is evaluated by expand_arith_string
//! (arrayfunc.c:1356 array_expand_index -> subst.c:3981) in a forked
//! child (subst.c:7143), never by the parent's unset-name walk, and an
//! arithmetic failure inside it abandons parameter_brace_expand BEFORE
//! subst.c:8216 parameter_brace_expand_error can run.
//!
//! #3540: `set -u; : ${b[$(echo 5)]}` runs the comsub, then reports
//! `b[$(echo 5)]: unbound variable` — the parameter text, not `echo`
//! (the pre-scan split the subscript text and reported the comsub's
//! first command name).
//!
//! #3541: `${a[$(echo E; echo 5)]:?err}` emits the subscript's arithmetic
//! diagnostic and aborts; the `:?` report must not follow it.

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    #[cfg(unix)]
    std::os::unix::process::CommandExt::arg0(&mut command, "bash");
    let output = command
        .arg("-c")
        .arg(script)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// #3540: the comsub inside the subscript runs and the diagnostic names
/// the parameter text, not the comsub's first command.
#[test]
fn comsub_subscript_reports_parameter_text() {
    // Under `-c` the unbound fatal (DISCARD) aborts the whole line —
    // `done` never prints; GNU rc=127.
    let (stdout, stderr, status) = rubash("set -u; : ${b[$(echo 5)]}; echo done\n");
    assert_eq!(stdout, "", "stdout={stdout:?}");
    assert_eq!(status, Some(127), "status={status:?}");
    assert!(
        stderr.ends_with("b[$(echo 5)]: unbound variable\n"),
        "stderr={stderr:?}"
    );
    assert!(
        !stderr.contains("echo: unbound variable"),
        "stderr={stderr:?}"
    );
}

/// #3540: the comsub's side effects (child stderr) precede the fatal.
#[test]
fn comsub_subscript_runs_before_fatal() {
    let (_, stderr, _) = rubash("set -u; : ${b[$(echo 5 >&2; echo 5)]}\n");
    assert!(
        stderr.starts_with("5\n"),
        "comsub stderr first: stderr={stderr:?}"
    );
    assert!(
        stderr.contains("b[$(echo 5 >&2; echo 5)]: unbound variable"),
        "stderr={stderr:?}"
    );
}

/// #3541: a subscript arithmetic error aborts before the `:?` report.
#[test]
fn subscript_arith_error_suppresses_question_report() {
    let (_stdout, stderr, status) = rubash("a=(x); : ${a[$(echo E; echo 5)]:?err}; echo st=$?\n");
    assert!(
        stderr.contains("arithmetic syntax error"),
        "stderr={stderr:?}"
    );
    assert!(
        !stderr.contains(": err"),
        "no `:?` report after arith error: stderr={stderr:?}"
    );
    // The aborting span is inside `( ... )` only in the issue's
    // reproducer; here the failing expansion aborts the -c line, so
    // `st=1` must come from the subshell form.
    let (stdout, _stderr2, _status) =
        rubash("a=(x); ( : ${a[$(echo E; echo 5)]:?err} ); echo st=$?\n");
    assert_eq!(stdout, "st=1\n", "stdout={stdout:?}");
    let _ = status;
}
