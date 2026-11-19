//! Issue rubash#3428 — `[[ ]]` redirect-open failure reports the `]]`
//! closing token's line in the READER's numbering: a backslash-newline
//! continuation inside the conditional does not advance it, while a
//! real newline does. The arith `(( ))` family intentionally keeps the
//! physical completion line (rubash#3406) — the two are asymmetric in
//! GNU, and each keeps its own convention.

use std::process::Command;

fn rubash(script: &str) -> (String, String) {
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
    )
}

/// Continuation-joined `[[ x == 1 \`<newline>` ]] < BAD` reports the
/// logical (continuation-merged) line 1.
#[test]
fn cond_continuation_redirect_reports_start_line() {
    let (stdout, stderr) = rubash("[[ x == 1 \\\n]] < /nonexistent-file-xyz\necho after\n");
    assert_eq!(stdout, "after\n", "stdout={stdout:?}");
    assert!(
        stderr.contains("line 1: /nonexistent-file-xyz: No such file or directory"),
        "stderr={stderr:?}"
    );
    assert!(!stderr.contains("line 2:"), "stderr={stderr:?}");
}

/// The same `[[ ]]` split by a REAL newline still reports the
/// completion line (2).
#[test]
fn cond_real_newline_redirect_reports_completion_line() {
    let (stdout, stderr) = rubash("[[ a == a &&\nb == b ]] < /nonexistent-file-xyz\necho after\n");
    assert_eq!(stdout, "after\n", "stdout={stdout:?}");
    assert!(
        stderr.contains("line 2: /nonexistent-file-xyz: No such file or directory"),
        "stderr={stderr:?}"
    );
}

/// Guard: the arith family keeps PHYSICAL completion on a
/// continuation-joined `(( ... ))` (rubash#3406 stays green).
#[test]
fn arith_continuation_redirect_still_reports_physical_line() {
    let (stdout, stderr) = rubash("(( x = 1 \\\n)) < /nonexistent-file-xyz\necho after\n");
    assert_eq!(stdout, "after\n", "stdout={stdout:?}");
    assert!(
        stderr.contains("line 2: /nonexistent-file-xyz: No such file or directory"),
        "stderr={stderr:?}"
    );
}
