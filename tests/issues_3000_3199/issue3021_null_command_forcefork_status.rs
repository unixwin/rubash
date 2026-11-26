//! rubash#3021 — a null command (no command word after expansion) whose
//! redirections force GNU's fork returned the command substitution's
//! status instead of the forked child's redirection status.
//!
//! GNU `execute_cmd.c:4203 execute_null_command`: the redirect list is
//! scanned for `REDIR_VARASSIGN` or a redirector descriptor of 0
//! (`execute_cmd.c:4209-4214`, any instruction: heredoc, here-string,
//! `<`, `<>`, `<&`, `<&-`, `0>`). When one is present the shell forks;
//! the child only runs `do_redirections` and exits `EXECUTION_SUCCESS`
//! or `EXECUTION_FAILURE` (`execute_cmd.c:4244-4247`), and the parent's
//! status is that child's (`execute_cmd.c:982 wait_for`). Only the
//! in-shell branch returns `last_command_subst_status`
//! (`execute_cmd.c:4268-4276`). So `$(false) <<EOF` is status 0 while
//! `$(false) >/dev/null` stays 1.
//!
//! Expectations are the WSL GNU bash 5.3.20 oracle's, probed from script
//! files.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn rubash_file(name: &str, script: &str) -> (String, Option<i32>) {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash-issue3021-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create test dir");
    let script_path = dir.join("probe.sh");
    fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .output()
        .expect("run rubash script file");
    let _ = fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.code(),
    )
}

fn check(name: &str, script: &str, expected: &str) {
    let (stdout, _) = rubash_file(name, script);
    assert_eq!(stdout, expected, "case {name}: script {script:?}");
}

#[test]
fn issue_repro_comsub_word_with_heredoc_is_status_0() {
    // The mvdan/sh t0112/t0113 shape from the issue.
    check(
        "repro",
        "$(\nfalse\n) <<EOF\nbar\nEOF\necho rc=$?\n",
        "rc=0\n",
    );
    check(
        "repro-flat",
        "$(false) <<EOF\nbar\nEOF\necho rc=$?\n",
        "rc=0\n",
    );
}

#[test]
fn assignment_with_fd0_redirect_discards_substitution_status() {
    check(
        "assign-heredoc",
        "x=$(false) <<EOF\nbar\nEOF\necho rc=$?\n",
        "rc=0\n",
    );
    check(
        "assign-herestring",
        "x=$(exit 4) <<<a; echo rc=$?\n",
        "rc=0\n",
    );
    check(
        "assign-input",
        "x=$(exit 3) </dev/null; echo rc=$?\n",
        "rc=0\n",
    );
    check(
        "assign-tabheredoc",
        "x=$(exit 3) <<-EOF\n\ta\n\tEOF\necho rc=$?\n",
        "rc=0\n",
    );
}

#[test]
fn every_fd0_redirector_instruction_forces_the_fork() {
    // execute_cmd.c:4213 tests `rd->redirector.dest == 0` for every
    // instruction, not only the input ones.
    for (name, redirect) in [
        ("out0", "0>/dev/null"),
        ("dupin0", "0<&1"),
        ("rw", "<>/dev/null"),
        ("rw0", "0<>/dev/null"),
        ("close0", "0>&-"),
        ("closein", "<&-"),
    ] {
        check(
            name,
            &format!("x=$(exit 3) {redirect}; echo rc=$?\n"),
            "rc=0\n",
        );
    }
}

#[test]
fn non_fd0_redirectors_keep_the_substitution_status() {
    // The in-shell branch: last_command_subst_status survives.
    for (name, redirect) in [
        ("out", ">/dev/null"),
        ("err", "2>/dev/null"),
        ("rw1", "1<>/dev/null"),
        ("heredoc3", "3<<EOF\na\nEOF\n"),
        ("herestring3", "3<<<a"),
        ("input3", "3</dev/null"),
    ] {
        let script = if redirect.ends_with('\n') {
            format!("x=$(exit 3) {redirect}echo rc=$?\n")
        } else {
            format!("x=$(exit 3) {redirect}; echo rc=$?\n")
        };
        check(name, &script, "rc=3\n");
    }
    check("bare", "$(false)\necho rc=$?\n", "rc=1\n");
    check("bare-assign", "x=$(false)\necho rc=$?\n", "rc=1\n");
    check("bare-out", "$(false) >/dev/null\necho rc=$?\n", "rc=1\n");
}

#[test]
fn failed_fd0_redirect_still_reports_failure() {
    check(
        "fail-input",
        "x=$(exit 5) </nonexistent_issue3021 2>/dev/null; echo rc=$?\n",
        "rc=1\n",
    );
}

#[test]
fn forced_fork_status_drives_err_trap_and_errexit() {
    // The discarded status is not a failure: no ERR trap, no errexit.
    check(
        "err-trap",
        "trap 'echo ERR' ERR\n$(false) <<<a; echo rc=$?\n$(false) >/dev/null; echo rc=$?\n",
        "rc=0\nERR\nrc=1\n",
    );
    check(
        "errexit",
        "set -e\nx=$(false) 0>/dev/null; echo survived\nx=$(false) >/dev/null; echo notreached\n",
        "survived\n",
    );
    check(
        "function",
        "f() { x=$(exit 4) <&-; }; f; echo f=$?\n",
        "f=0\n",
    );
}
