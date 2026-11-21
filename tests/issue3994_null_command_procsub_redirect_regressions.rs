//! Issue rubash#3994: process substitution redirects on a command-less
//! assignment (or bare redirect-only null command) must expand and
//! materialize the process substitution rather than opening `<(...)` /
//! `>(...)` as a literal filename (ENOENT).
//!
//! GNU execute_cmd.c:4628-4635 (`execute_simple_command`) ->
//! `execute_null_command` (`execute_cmd.c:4203-4278`) ->
//! `do_redirections` (`redir.c:237`) -> `redirection_expand`
//! (`redir.c:298`) -> `expand_word_internal` -> `process_substitute`
//! (`subst.c:6362`).
//!
//! Expectations verified against WSL GNU Bash 5.3.20 via script-file
//! probes.

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue3994-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue3994_assignment_input_procsub_redirect_succeeds_silently() {
    let (stdout, stderr, code) = rubash_file("x=1 < <(sleep 0.1)\necho \"rc=$? x=$x\"\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "", "unexpected stderr: {stderr:?}");
    assert_eq!(stdout, "rc=0 x=1\n");
}

#[test]
fn issue3994_assignment_output_procsub_redirect_succeeds_silently() {
    let (stdout, stderr, code) = rubash_file("x=1 > >(cat >/dev/null)\necho \"rc=$? x=$x\"\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "", "unexpected stderr: {stderr:?}");
    assert_eq!(stdout, "rc=0 x=1\n");
}

#[test]
fn issue3994_bare_null_command_procsub_redirects_succeed_silently() {
    let (stdout, stderr, code) =
        rubash_file("< <(echo hi)\necho \"in=$?\"\n> >(cat >/dev/null)\necho \"out=$?\"\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "", "unexpected stderr: {stderr:?}");
    assert_eq!(stdout, "in=0\nout=0\n");
}

#[test]
fn issue3994_pipeline_brace_group_assignment_with_stdin_and_procsub_redirect() {
    let (stdout, stderr, code) =
        rubash_file("echo hi | { x=$(</dev/stdin) < <(sleep 0.1); echo \"inside=$x rc=$?\"; }\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "", "unexpected stderr: {stderr:?}");
    assert_eq!(stdout, "inside=hi rc=0\n");
}

#[test]
fn issue3994_assignment_procsub_redirect_executes_body_side_effects() {
    let (stdout, stderr, code) = rubash_file(
        "x=1 < <(touch mark_in; echo hi)\n\
         sleep 0.2\n\
         echo \"in_rc=$? mark=$(test -f mark_in && echo yes || echo no)\"\n\
         x=2 > >(cat > mark_out)\n\
         sleep 0.2\n\
         echo \"out_rc=$? out=$(test -f mark_out && echo yes || echo no)\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "", "unexpected stderr: {stderr:?}");
    assert_eq!(stdout, "in_rc=0 mark=yes\nout_rc=0 out=yes\n");
}

#[test]
fn issue3994_assignment_dup_input_procsub_is_ambiguous_redirect() {
    let (stdout, stderr, code) = rubash_file("x=1 <& <(echo hi)\necho \"rc=$?\"\n");
    assert_eq!(code, Some(0));
    assert!(
        stderr.contains("<(echo hi): ambiguous redirect"),
        "expected ambiguous redirect diagnostic, got: {stderr:?}"
    );
    assert_eq!(stdout, "rc=1\n");
}

#[test]
fn issue3994_quoted_procsub_redirect_on_assignment_remains_literal_enoent() {
    let (stdout, stderr, code) = rubash_file("x=1 < \"<(sleep 0.1)\"\necho \"rc=$?\"\n");
    assert_eq!(code, Some(0));
    assert!(
        stderr.contains("<(sleep 0.1): No such file or directory"),
        "expected ENOENT diagnostic for quoted literal, got: {stderr:?}"
    );
    assert_eq!(stdout, "rc=1\n");
}

#[test]
fn issue3994_assignment_procsub_followed_by_missing_file_still_fails() {
    let (stdout, stderr, code) =
        rubash_file("x=1 < <(echo hi) < /nonexistent_3994\necho \"rc=$?\"\n");
    assert_eq!(code, Some(0));
    assert!(
        stderr.contains("/nonexistent_3994: No such file or directory"),
        "expected ENOENT diagnostic for /nonexistent_3994, got: {stderr:?}"
    );
    assert_eq!(stdout, "rc=1\n");
}
