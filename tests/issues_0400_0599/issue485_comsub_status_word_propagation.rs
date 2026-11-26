//! Issue rubash#485: a command substitution's exit status must be visible
//! to `$?` in words to the RIGHT of the substitution within the SAME
//! command's word list — `echo "$(cd nope && pwd)" $?` prints 1 under GNU
//! Bash, 0 under rubash before the fix.
//!
//! GNU spec (subst.c command_substitute): the parent wait_for()s the
//! substitution child and records the wait status in
//! last_command_exit_value — the same rule the nofork funsub form
//! documents at subst.c:7120-7122. Each substitution overwrites the
//! previous one left-to-right (`echo $(true) $(false) $?` is 1,
//! `echo $(false) $(true) $?` is 0), and a SUCCEEDING substitution resets
//! a failing previous status (`false; echo $(true) $?` is 0). The record
//! is bookkeeping only: the enclosing command still runs and imposes its
//! own exit status, so errexit semantics are unchanged (`set -e; echo
//! "$(false)"; echo after` keeps running) and `local x="$(false)"` stays
//! 0 (the local builtin's own status).
//!
//! The case head gets the same persistence: the case command commits no
//! status until a clause body finishes (execute_cmd.c execute_case_command),
//! so `case $(false) in "") echo $?;; esac` prints 1.
//!
//! Rust semantic owner: command_substitution.rs
//! expand_command_substitution_with_context (word_expansion_comsub_exit
//! overlay read by dollar_question_status), the echo fast path pinning the
//! substitution cell back to 0 (`$(echo $(false))` is status 0), the
//! cd/pwd shortcut emitting the swallowed cd diagnostic, compound_exec.rs
//! execute_case_command committing the overlay before the clause walk, and
//! ast_exec.rs clearing the overlay at each command-node boundary.

#![cfg(windows)]

use std::io::Write;
use std::process::{Command, Stdio};

struct RunOutcome {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_rubash_script(script: &str) -> RunOutcome {
    let dir = std::env::temp_dir().join(format!("rubash-issue485-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue485 scratch dir");
    let script_path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&script_path).expect("write probe script");
    file.write_all(script.as_bytes())
        .expect("write probe script");
    drop(file);
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash script");
    let _ = std::fs::remove_dir_all(&dir);
    RunOutcome {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// The issue's reproducer: comsub-internal cd failure, $? to the right.
/// GNU prints the cd diagnostic (script line prefix) and ` 1`.
#[test]
fn comsub_cd_failure_visible_in_trailing_dollar_question() {
    let out = run_rubash_script("echo \"$(cd nope && pwd)\" $?\n");
    assert_eq!(out.code, Some(0));
    assert!(
        out.stderr
            .ends_with("cd: nope: No such file or directory\n"),
        "stderr: {:?}",
        out.stderr
    );
    assert_eq!(out.stdout, " 1\n");
}

/// Unquoted form with an internal failure.
#[test]
fn comsub_false_visible_in_trailing_dollar_question() {
    let out = run_rubash_script("echo $(false) $?\n");
    assert_eq!(out.code, Some(0));
    // Unquoted $(false) expands to zero words, so echo prints only the $?
    // value (GNU: `1` with no leading space).
    assert_eq!(out.stdout, "1\n");
}

/// Quoted substitution form.
#[test]
fn quoted_comsub_false_visible_in_trailing_dollar_question() {
    let out = run_rubash_script("echo \"$(false)\" $?\n");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, " 1\n");
}

/// Left-to-right: each substitution overwrites the previous status.
#[test]
fn later_comsub_overwrites_earlier_status() {
    let out = run_rubash_script("echo $(true) $(false) $?\n");
    assert_eq!(out.stdout, "1\n");
    let out = run_rubash_script("echo $(false) $(true) $?\n");
    assert_eq!(out.stdout, "0\n");
}

/// A succeeding substitution resets a failing previous command's status.
#[test]
fn succeeding_comsub_resets_previous_failure() {
    let out = run_rubash_script("false\necho $(true) $?\n");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "0\n");
}

/// The overlay does not survive into the NEXT command: the intermediate
/// `true` commits its own status.
#[test]
fn overlay_does_not_leak_past_command_boundary() {
    let out = run_rubash_script("echo $(false) $?\ntrue\necho $?\n");
    assert_eq!(out.stdout, "1\n0\n");
}

/// Nested: the body's FINAL command decides the substitution's status, so
/// `$(echo $(false))` is 0 (GNU matrix t5 row A).
#[test]
fn nested_comsub_body_final_command_decides_status() {
    let out = run_rubash_script("echo $(echo $(false)) $?\n");
    assert_eq!(out.stdout, "0\n");
}

/// The `local x="$(false)"` shape stays 0 (rubash and GNU agree): the local
/// builtin's own status wins.
#[test]
fn local_assignment_comsub_failure_stays_zero() {
    let out = run_rubash_script("f() { local x=\"$(false)\"; echo $?; }\nf\n");
    assert_eq!(out.stdout, "0\n");
}

/// Plain assignment still reports the substitution's status (pre-existing
/// behavior, regression guard).
#[test]
fn plain_assignment_comsub_failure_reports_one() {
    let out = run_rubash_script("x=$(false); echo $?\n");
    assert_eq!(out.stdout, "1\n");
}

/// errexit is NOT triggered by a failing substitution inside a succeeding
/// command (GNU matrix t3: `echo after` still runs), but IS triggered by a
/// bare failing substitution (null command).
#[test]
fn errexit_semantics_unchanged() {
    let out = run_rubash_script("set -e\necho \"$(false)\"\necho reached\n");
    assert_eq!(out.code, Some(0));
    assert!(out.stdout.contains("reached\n"));
    let out = run_rubash_script("set -e\n$(false)\necho unreached\n");
    assert_eq!(out.code, Some(1));
    assert!(!out.stdout.contains("unreached"));
}

/// Case head: the case command commits no status until a clause body
/// finishes, so the head's substitution status is still $? inside the body.
#[test]
fn case_head_comsub_status_visible_in_clause_body() {
    let out = run_rubash_script("case $(false) in \"\") echo \"case: $?\";; esac\n");
    assert_eq!(out.stdout, "case: 1\n");
}

/// The echo fast path pins the substitution status to echo's own 0 even
/// when an argument carried a nested failing substitution.
#[test]
fn echo_fast_path_status_is_zero_over_nested_failure() {
    let out = run_rubash_script("x=$(echo $(false)); echo $?;\n");
    assert_eq!(out.stdout, "0\n");
}

/// ERR trap between commands still sees each command's committed status.
#[test]
fn err_trap_sees_committed_statuses() {
    let out = run_rubash_script("trap 'echo \"ERR:$?\"' ERR\nfalse\necho after\n");
    assert_eq!(out.stdout, "ERR:1\nafter\n");
}
