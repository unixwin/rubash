//! Issue rubash#846 — `!` must NOT flip a compound own redirect failure.
//!
//! GNU execute_cmd.c:854-883: a compound do_redirections failure returns
//! EXECUTION_FAILURE before the :1204-1207 invert step, so
//! `! { :; } > /bad` exits 1 (no flip). Two follow-on fixes are covered
//! here as well:
//!
//! - `! [[ ... ]] > /bad` takes a separate dispatch path
//!   (execute_conditional_command_with_redirects in command_prepare.rs)
//!   that never set the compound_redirect_failed mark, so it flipped to 0.
//! - A failing pipeline-stage output redirect (`! true | cat > /bad`)
//!   propagated Err past an active thread-local stdout capture without
//!   restoring it, swallowing every later stdout write in the script.
//!
//! Byte-verified against WSL GNU Bash 5.3.0 (script-file oracle).
//! /proc/nope846 is unwritable even for root, so the failure reproduces
//! for privileged and unprivileged users alike.

use std::process::Command;

const BAD: &str = "/proc/nope846";

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// #846 core: a brace group own redirect failure is not inverted.
#[test]
fn bang_brace_redirect_failure_does_not_flip() {
    let (stdout, stderr, code) = rubash(&format!("! {{ :; }} > {BAD}\necho brace=$?"));
    assert_eq!(stdout, "brace=1\n");
    assert!(stderr.contains("No such file or directory"), "{stderr:?}");
    assert_eq!(code, Some(0));
}

/// #846 follow-on: `[[ ]]` with redirects dispatches through
/// execute_conditional_command_with_redirects, which must set the
/// compound_redirect_failed mark like every other compound path.
#[test]
fn bang_cond_redirect_failure_does_not_flip() {
    let (stdout, stderr, _) = rubash(&format!("! [[ -n x ]] > {BAD}\necho cond=$?"));
    assert_eq!(stdout, "cond=1\n");
    assert!(stderr.contains("No such file or directory"), "{stderr:?}");
}

/// #846: the `(( ))` with-redirects path keeps its mark (late.rs arm).
#[test]
fn bang_arith_redirect_failure_does_not_flip() {
    let (stdout, stderr, _) = rubash(&format!("! (( 1 )) > {BAD}\necho arith=$?"));
    assert_eq!(stdout, "arith=1\n");
    assert!(stderr.contains("No such file or directory"), "{stderr:?}");
}

/// #846: subshell body own redirect failure is not inverted either.
#[test]
fn bang_subshell_redirect_failure_does_not_flip() {
    let (stdout, stderr, _) = rubash(&format!("! ( :; ) > {BAD}\necho subsh=$?"));
    assert_eq!(stdout, "subsh=1\n");
    assert!(stderr.contains("No such file or directory"), "{stderr:?}");
}

/// #846: the mark is left set on purpose — an outer `!` suppresses in
/// turn (the inner command never runs, both levels observe status 1).
#[test]
fn bang_bang_nested_redirect_failure_stays_failed() {
    let (stdout, _, _) = rubash(&format!("! ! {{ :; }} > {BAD}\necho bangbang=$?"));
    assert_eq!(stdout, "bangbang=1\n");
}

/// #846 clobber guard: a failing last-stage output redirect must not
/// swallow later stdout. The preflight Err used to propagate past the
/// active thread-local stdout capture without restoring it, so `rc=0`
/// and everything after vanished while stderr kept working.
#[test]
fn bang_pipeline_stage_redirect_failure_keeps_later_stdout() {
    let (stdout, stderr, _) = rubash(&format!("! true | cat > {BAD}\necho rc=$?\necho ALIVE"));
    assert_eq!(stdout, "rc=0\nALIVE\n");
    assert!(stderr.contains("No such file or directory"), "{stderr:?}");
}

/// #846 clobber guard without `!`: pipeline status is the last stage own
/// 1 and the script stdout after the pipeline is intact.
#[test]
fn plain_pipeline_stage_redirect_failure_keeps_later_stdout() {
    let (stdout, stderr, code) = rubash(&format!("true | cat > {BAD}\necho rc=$?\necho ALIVE2"));
    assert_eq!(stdout, "rc=1\nALIVE2\n");
    assert!(stderr.contains("No such file or directory"), "{stderr:?}");
    assert_eq!(code, Some(0));
}

/// #846 boundary: a subshell whose single body is the inverted command
/// flips at the region boundary (GNU execute_cmd.c:1831), so the
/// subshell as a whole exits 0.
#[test]
fn subshell_single_inverted_cond_body_flips_at_boundary() {
    let (stdout, _, _) = rubash(&format!("( ! [[ -n x ]] > {BAD} )\necho subcond=$?"));
    assert_eq!(stdout, "subcond=0\n");
}

/// #846 sanity: without `!` the `[[ ]]` redirect failure is a plain 1.
#[test]
fn plain_cond_redirect_failure_is_one() {
    let (stdout, stderr, _) = rubash(&format!("[[ -n x ]] > {BAD}\necho plaincond=$?"));
    assert_eq!(stdout, "plaincond=1\n");
    assert!(stderr.contains("No such file or directory"), "{stderr:?}");
}
