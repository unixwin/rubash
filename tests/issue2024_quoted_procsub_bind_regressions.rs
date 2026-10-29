//! rubash#2024 — the compound INPUT bind layer must not materialize a
//! quoted process-substitution target by VALUE.
//!
//! GNU redir.c / parse.y:
//! - A `<(`/`>(` span is recorded only when the parser saw the `<(`
//!   UNQUOTED (parse.y read_token_word); a QUOTED `"<(echo hi)"` redirect
//!   target is a literal filename.
//! - redir.c:916 do_redirection_internal → redir_open opens that name,
//!   fails ENOENT, and redirection_error (redir.c:135) prints
//!   `<(echo hi): No such file or directory`; the compound is abandoned
//!   with status 1 and the body never runs.
//! - The same holds for the indirect form (`t='<(echo hi)'; … < "$t"`):
//!   the expanded result is data, never a substitution.
//!
//! rubash used to match the bind layer (bind_command_fd0_input,
//! loop_redirect_input) on the quote-stripped target value and spawn the
//! quoted span's body. The fix first grew the bind layer's literal-target
//! error channel (compound_stdin_open_failure probing the literal path —
//! declining the spawn without the channel made the compound fall back to
//! the ambient stdin and hang), then switched the spawn arms to the same
//! recorded-span admission the simple-command path has used since #1931
//! (external_setup redirect_target_span_is_recorded). posix_errors
//! path_error maps the ERROR_INVALID_NAME a `<`/`>`-bearing literal
//! provokes on Windows to GNU's ENOENT text (a name no session file can
//! ever match).
//!
//! All expectations measured against GNU bash 5.3.20 with script files
//! (WSL /usr/local/bin/bash, 2026-10-29).

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2024-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = tempdir(tag);
    let case = dir.join("case.sh");
    std::fs::write(&case, script).expect("write fixture");
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

#[test]
fn issue2024_brace_group_quoted_procsub_target_is_literal() {
    let (stdout, stderr, code) = run(
        "brace",
        "{ echo body-ran; } < \"<(echo hi)\"\necho \"rc=$?\"\n",
    );
    assert!(
        !stdout.contains("body-ran"),
        "the body must not run: stdout: {stdout}"
    );
    assert_eq!(stdout, "rc=1\n", "stdout: {stdout}");
    assert!(
        stderr.contains("<(echo hi): No such file or directory"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0), "the script itself completes");
}

#[test]
fn issue2024_loop_quoted_procsub_target_is_literal() {
    let (stdout, stderr, code) = run(
        "loop",
        "while read x; do echo \"got:$x\"; done < \"<(echo hi)\"\necho \"rc=$?\"\n",
    );
    assert!(!stdout.contains("got:hi"), "stdout: {stdout}");
    assert_eq!(stdout, "rc=1\n", "stdout: {stdout}");
    assert!(
        stderr.contains("<(echo hi): No such file or directory"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue2024_indirect_quoted_target_is_literal() {
    let (stdout, stderr, code) = run(
        "indirect",
        "t='<(echo hi)'\n{ echo body-ran; } < \"$t\"\necho \"rc1=$?\"\nwhile read x; do echo \"got:$x\"; done < \"$t\"\necho \"rc2=$?\"\n",
    );
    assert!(!stdout.contains("body-ran"), "stdout: {stdout}");
    assert!(!stdout.contains("got:"), "stdout: {stdout}");
    assert_eq!(stdout, "rc1=1\nrc2=1\n", "stdout: {stdout}");
    assert!(
        stderr.contains("<(echo hi): No such file or directory"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue2024_unquoted_procsub_target_still_spawns() {
    let (stdout, stderr, code) = run(
        "control",
        "while read x; do echo \"got:$x\"; done < <(echo hi)\necho \"rc=$?\"\n",
    );
    assert_eq!(stdout, "got:hi\nrc=0\n", "stdout: {stdout}");
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue2024_exec_face_quoted_procsub_target_is_literal() {
    let (stdout, stderr, code) = run("exec", "exec cat < \"<(echo hi)\"\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n", "stdout: {stdout}");
    assert!(
        stderr.contains("<(echo hi): No such file or directory"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue2024_builtin_face_quoted_procsub_target_is_literal() {
    let (stdout, stderr, code) = run("builtin", "read x < \"<(echo hi)\"\necho \"rc=$? x=$x\"\n");
    assert_eq!(stdout, "rc=1 x=\n", "stdout: {stdout}");
    assert!(
        stderr.contains("<(echo hi): No such file or directory"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue2024_set_u_unbound_loop_target_does_not_hang() {
    // GNU bash 5.3.20 reports `NOPE: unbound variable` here and terminates
    // the script; the #2024 acceptance for this form is that the bind
    // level FAILS instead of reading the ambient stdin (the pre-fix naive
    // guard hung exactly there). The unbound-variable wording itself is a
    // parameter-expansion gap measured on master and out of #2024's
    // channel scope.
    let (stdout, stderr, code) = run(
        "nounset",
        "set -u\nwhile read x; do echo \"got:$x\"; done < \"$NOPE\"\necho \"rc=$?\"\n",
    );
    assert!(!stdout.contains("got:"), "stdout: {stdout}");
    assert_eq!(
        stdout, "rc=1\n",
        "the compound aborts and the script continues"
    );
    assert!(
        stderr.contains("No such file or directory"),
        "the literal-target channel reports the failed open: stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}
