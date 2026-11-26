//! Issue rubash#485: a command substitution's exit status must be the
//! shell's `$?` the moment the substitution completes, so a later `$?` on
//! the same command line (later word, later fragment of the same word,
//! later assignment RHS on the same simple command) sees it — rubash kept
//! the pre-expansion status (GNU 1, rubash 0 for the issue's repro).
//!
//! GNU spec (subst.c command_substitute): the parent reaps the substitution
//! child and the child's exit status becomes last_command_exit_value
//! BEFORE the enclosing command runs; the enclosing command's own status
//! then overwrites it. Oracle probes, WSL GNU bash 5.3.0
//! (/usr/local/bin/bash, 2026-10-10) — every literal below is the GNU
//! output:
//!
//! ```text
//! $ echo "$(cd nope && pwd)" $?        # issue repro (cd diagnostic on stderr)
//! bash: line 1: cd: nope: No such file or directory
//!  1
//! $ echo "$(exit 3)" $?                # quoted whole word
//!  3
//! $ false; echo "$(true) $?"           # a SUCCESSFUL comsub clobbers too
//!  0
//! $ true; echo $? "$(exit 3)"          # left-to-right: $? BEFORE sees old
//! 0
//! $ echo "$(exit 3)$?"                 # same-word trailing fragment
//! 3
//! $ echo `exit 3` $?                   # backtick word
//!  3
//! $ v="$(exit 3)$?"; echo "$v $?"      # assignment: value 3, status 3
//! 3 3
//! $ f(){ local v="$(exit 3)$?"; echo "$v $?"; }; f
//! 3 0                                  # declaration builtin masks (local rc 0)
//! ```
//!
//! set -e interaction (D case, pre-existing and preserved): an assignment
//! whose RHS comsub fails still aborts with rc 1, while a WORD-attached
//! comsub failure does NOT abort (the command — echo — succeeds).
//!
//! Rust semantic owners: executor/command_substitution.rs
//! expand_command_substitution_with_context (status cell), executor/
//! embedded_mutations.rs run_ast_command_substitution_with_context /
//! run_function_command_substitution / expand_command_substitution_mut_
//! typed_with_context (parent-side reap: exit_code = substitution status),
//! executor/command_prepare.rs the whole-word quoted `$()` fast path, and
//! executor/parameter_core.rs expand_word_mut_with_context (embedded
//! backtick words route to the mutable walker).

#![cfg(windows)]

use std::io::Write;
use std::process::{Command, Stdio};

fn run_rubash_script(script: &str) -> (Option<i32>, String) {
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
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

/// The issue's repro: `echo "$(cd nope && pwd)" $?` — the failed cd inside
/// the substitution is visible as `$?` when the NEXT word expands. GNU
/// prints " 1" (empty substitution, $? = 1). The cd diagnostic goes to
/// stderr and is not asserted here.
#[test]
fn issue_repro_cd_failure_visible_to_later_word() {
    let (code, stdout) = run_rubash_script("echo \"$(cd nope && pwd)\" $?\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, " 1\n");
}

/// Same shape, failed exit status instead of a failed cd.
#[test]
fn quoted_whole_word_comsub_status_visible_to_later_word() {
    let (code, stdout) = run_rubash_script("echo \"$(exit 3)\" $?\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, " 3\n");

    let (code, stdout) = run_rubash_script("echo \"$(exit 3)\" \"$?\"\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, " 3\n");

    // Unquoted word-attached comsub: same visibility after field splitting.
    // The unquoted empty comsub word is removed (null expansion), so the
    // line prints "3" while $? still expanded to 3.
    let (code, stdout) = run_rubash_script("echo $(exit 3) $?\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "3\n");
}

/// A SUCCESSFUL substitution clobbers $? just the same: after `false`, the
/// comsub's 0 is what a later word sees. GNU: `false; echo "$(true) $?"`
/// prints " 0" (subst.c reap contract is unconditional).
#[test]
fn successful_comsub_clobbers_status_for_later_word() {
    let (code, stdout) = run_rubash_script("false; echo \"$(true) $?\"\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, " 0\n");
}

/// Left-to-right word order: a `$?` that expands BEFORE the substitution
/// keeps the pre-command status. GNU: `true; echo $? "$(exit 3)"` prints
/// "0 " (the trailing comsub output is empty).
#[test]
fn dollar_question_before_comsub_sees_previous_status() {
    let (code, stdout) = run_rubash_script("true; echo $? \"$(exit 3)\"\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "0 \n");

    let (code, stdout) = run_rubash_script("false; echo $? $(true)\n");
    assert_eq!(code, Some(0));
    // The unquoted empty $(true) word is removed after expansion.
    assert_eq!(stdout, "1\n");
}

/// Fragment shapes: the comsub and the trailing `$?` inside ONE word.
#[test]
fn same_word_trailing_fragment_sees_comsub_status() {
    let (code, stdout) = run_rubash_script("echo \"$(exit 3)$?\"\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "3\n");

    let (code, stdout) = run_rubash_script("false; echo \"pre$(true)post $?\"\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "prepost 0\n");
}

/// Last-comsub-wins: two substitutions on one line, the later one's status
/// is what the trailing `$?` sees. GNU: "  0".
#[test]
fn later_comsub_status_wins() {
    let (code, stdout) = run_rubash_script("echo \"$(exit 3)\" \"$(true)\" $?\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "  0\n");
}

/// Backtick words carry the same contract (the typed whole-word path and
/// the mutable embedded walker).
#[test]
fn backtick_comsub_status_visible_to_later_word() {
    let (code, stdout) = run_rubash_script("echo `exit 3` $?\n");
    assert_eq!(code, Some(0));
    // The unquoted empty backtick word is removed after expansion.
    assert_eq!(stdout, "3\n");

    let (code, stdout) = run_rubash_script("echo a`exit 3`b $?\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "ab 3\n");
}

/// Assignments: the value still concatenates in expansion order and the
/// assignment's own status stays the LAST comsub's status. GNU:
/// `v="$(exit 3)$?"` leaves v=3 with status 3.
#[test]
fn assignment_value_and_status_keep_last_comsub() {
    let (code, stdout) = run_rubash_script("v=\"$(exit 3)$?\"; echo \"$v $?\"\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "3 3\n");

    // Multi-assignment simple command: the second RHS's $? sees the first
    // RHS's comsub status. GNU: `false; x=$(true) y=$?` gives y=0, rc 0.
    let (code, stdout) = run_rubash_script("false; x=$(true) y=$?; echo \"$y $?\"\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "0 0\n");
}

/// Plain assignment status propagation is unchanged: `x=$(exit 7)` leaves 7.
#[test]
fn plain_assignment_status_unchanged() {
    let (code, stdout) = run_rubash_script("x=$(exit 7); echo $?\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "7\n");

    // The command's own status still overwrites: echo succeeds.
    let (code, stdout) =
        run_rubash_script("echo $(false); echo $?; echo \"$(exit 9) x\"; echo $?\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "\n0\n x\n0\n");
}

/// Declaration builtins mask the assignment status (GNU: local returns 0)
/// while the VALUE still sees the promoted $? inside the RHS.
#[test]
fn local_masks_status_but_value_sees_promotion() {
    let (code, stdout) = run_rubash_script("f() { local v=\"$(exit 3)$?\"; echo \"$v $?\"; }; f\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "3 0\n");
}

/// D case (set -e, pre-existing GNU parity — must NOT regress): a failed
/// comsub assignment under `set -e` aborts with rc 1.
#[test]
fn set_e_comsub_assignment_failure_aborts() {
    let (code, stdout) = run_rubash_script("set -e\nx=$(false)\necho unreachable\n");
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "");

    let (code, stdout) = run_rubash_script("set -e\nx=$(exit 4)\necho unreachable\n");
    assert_eq!(code, Some(4));
    assert_eq!(stdout, "");
}

/// set -e does NOT abort on a word-attached comsub failure: the command
/// (echo) succeeds, and the comsub's status is only visible mid-line.
#[test]
fn set_e_word_attached_comsub_failure_does_not_abort() {
    let (code, stdout) = run_rubash_script("set -e\necho \"$(exit 9) x\"\necho after $?\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, " x\nafter 0\n");
}

/// Nested substitutions: the inner comsub's status is the outer body's $? —
/// the outer comsub (a subshell) reports its own body's last status to the
/// parent. GNU: `echo "$(echo $(exit 3)) $?"` prints " 0".
#[test]
fn nested_comsub_outer_body_status_wins() {
    let (code, stdout) = run_rubash_script("echo \"$(echo $(exit 3)) $?\"\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, " 0\n");
}
