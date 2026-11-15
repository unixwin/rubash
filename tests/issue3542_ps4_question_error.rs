//! Issue rubash#3542 — PS4 expands through the same containment channel
//! as PS1/`${var@P}`: GNU expand_prompt_string (subst.c:4392-4418) runs
//! the pass under no_longjmp_on_fatal_error and falls back to the
//! decoded string on any verdict, so a `:?` error operator inside PS4
//! REPORTS and leaves the prompt text literal — the message is never
//! substituted into the prefix.

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

#[test]
fn ps4_question_error_reports_and_stays_literal() {
    let (stdout, stderr, _status) = rubash("( PS4='+${q:?err}'; set -x; : hi ); echo st=$?\n");
    assert_eq!(stdout, "st=0\n", "stdout={stdout:?}");
    assert!(
        stderr.contains("q: err"),
        "diagnostic reported: stderr={stderr:?}"
    );
    assert!(
        stderr.contains("+${q:?err}: hi"),
        "prompt falls back to the literal text: stderr={stderr:?}"
    );
    assert!(
        !stderr.contains("+err: hi"),
        "message must not substitute: stderr={stderr:?}"
    );
}

/// Guard: an unset-but-no-operator PS4 expansion still works, and a
/// bound `${q}` still expands.
#[test]
fn ps4_normal_expansion_unchanged() {
    let (_stdout, stderr, _status) = rubash("( q=ok; PS4='+${q}: '; set -x; : hi )\n");
    assert!(stderr.contains("+ok: : hi"), "stderr={stderr:?}");
}
