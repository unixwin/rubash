//! rubash#2761 — heredoc bodies on raw-empty null commands were never
//! expanded: a command with redirections but no words at all (a bare
//! `<<EOF`, or `3<<EOF`) went straight from the empty-words arm to
//! `execute_empty_words_command`, bypassing the stdin pre-expansion
//! funnel (`preexpand_command_stdin`, the GNU do_redirections point)
//! every word-bearing command passes through. Command substitutions in
//! the body never ran and `:=` never bound.
//!
//! The fix routes the bodies through that same funnel inside
//! `execute_empty_words_command`, after the null command's assignments
//! are bound and after the adjudicated assignment-error arms
//! (#2369/#1542/#402) have had their chance to discard the command —
//! the expansion goes THROUGH that error handling, not around it. The
//! funnel classifies the context itself (#2681): an fd-0 heredoc forces
//! the fork boundary (`null_command_redirects_force_fork`), so its
//! effects run but do not persist; a numbered-fd body expands in the
//! shell, so its bindings persist.
//!
//! Expectations verified against GNU bash 5.3.20 per the issue. NOTE on
//! the box oracle: this machine's GNU is 5.2.21, where a bare fd-0
//! heredoc still binds `:=` in the shell — bash 5.3 changed the
//! null-command force-fork to cover it. Rubash follows the #2681
//! force-fork model (the issue's 5.3.20 oracle), so the fd-0 test
//! asserts non-persistence here.

use std::process::Command;

fn run_script(name: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-issue2761-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let script_path = dir.join("case.sh");
    std::fs::write(&script_path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let result = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn fd0_bare_heredoc_body_expands_but_does_not_persist() {
    let (stdout, stderr, code) = run_script(
        "fd0-expands-forked",
        "unset v\n<<EOF\n$(: > se)\n${v:=bound}\nEOF\necho \"fd0: se=$(test -e se && echo yes || echo no) v=${v:-UNSET}\"\n",
    );
    assert_eq!(stdout, "fd0: se=yes v=UNSET\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn fd3_bare_heredoc_body_expands_and_persists() {
    let (stdout, stderr, code) = run_script(
        "fd3-expands-shell",
        "unset v\n3<<EOF\n$(: > se)\n${v:=bound}\nEOF\necho \"fd3: se=$(test -e se && echo yes || echo no) v=${v:-UNSET}\"\n",
    );
    assert_eq!(stdout, "fd3: se=yes v=bound\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn quoted_delimiter_bare_heredoc_body_stays_literal() {
    let (stdout, stderr, code) = run_script(
        "quoted-stays-literal",
        "unset v\n<<'EOF'\n$(: > se)\n${v:=bound}\nEOF\necho \"quoted: se=$(test -e se && echo yes || echo no) v=${v:-UNSET}\"\n",
    );
    assert_eq!(stdout, "quoted: se=no v=UNSET\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn readonly_assignment_discards_before_body_expands() {
    // #2369/#1542: the assignment-error arm discards the null command
    // before do_redirections, so the body must never expand — the
    // side-effect file stays absent even though the diagnostic, the
    // status, and the following line are all GNU's. (GNU 5.2.21 on the
    // box gives rc=1 for this exact shape, plain `ro=x` included;
    // verified, not assumed.)
    let (stdout, stderr, code) = run_script(
        "readonly-discards-first",
        "readonly ro\nro=x <<EOF\n$(: > se)\nEOF\nrc=$?\necho \"after: se=$(test -e se && echo yes || echo no) rc=$rc\"\n",
    );
    assert_eq!(stdout, "after: se=no rc=1\n");
    assert!(
        stderr.contains("ro: readonly variable"),
        "stderr was: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn body_sees_null_command_assignments() {
    // GNU expand_words binds the null command's assignments before
    // do_redirections, so the body expands against the post-binding
    // values — the pre-expansion must sit after the assignment loop.
    let (stdout, stderr, code) = run_script(
        "body-sees-assignments",
        "x=hello <<EOF\n$(echo \"$x\" > out)\nEOF\necho \"assign-visible: $(cat out)\"\n",
    );
    assert_eq!(stdout, "assign-visible: hello\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn bare_here_string_side_effect_runs() {
    // The here-string word is expanded by the same funnel: a command
    // substitution inside it runs.
    let (stdout, stderr, code) = run_script(
        "bare-here-string-se",
        "<<<\"$(: > se)\"\necho \"herestring: se=$(test -e se && echo yes || echo no)\"\n",
    );
    assert_eq!(stdout, "herestring: se=yes\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn bare_here_string_binding_does_not_persist() {
    // A bare here-string targets fd 0, so the null-command force-fork
    // rule (#2681) expands it in the fork: `:=` does not persist —
    // the same 5.2-vs-5.3 split as the fd-0 heredoc above (GNU 5.2.21
    // binds here too; rubash follows the force-fork model).
    let (stdout, stderr, code) = run_script(
        "bare-here-string-bind",
        "unset v\n<<<\"${v:=bound}\"\necho \"herestring: v=${v:-UNSET}\"\n",
    );
    assert_eq!(stdout, "herestring: v=UNSET\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn vanished_words_null_command_still_expands_body() {
    // Words that expand to nothing still dispatch as a null command
    // (#3021: the fork discards the vanished substitution's status,
    // so the command is status 0) — and its body still expands.
    let (stdout, stderr, code) = run_script(
        "vanished-words",
        "$(false) <<EOF\n$(: > se)\nEOF\necho \"vanished: rc=$? se=$(test -e se && echo yes || echo no)\"\n",
    );
    assert_eq!(stdout, "vanished: rc=0 se=yes\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn worded_heredoc_neighbor_still_expands() {
    // Control: the word-bearing path is untouched — an external
    // command's body expands in the fork (side effect runs, `:=`
    // discarded), exactly as before.
    let (stdout, stderr, code) = run_script(
        "worded-neighbor",
        "unset v\ncat <<EOF >/dev/null\n$(: > se)\n${v:=bound}\nEOF\necho \"cat: se=$(test -e se && echo yes || echo no) v=${v:-UNSET}\"\n",
    );
    assert_eq!(stdout, "cat: se=yes v=UNSET\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn fd3_funsub_exit_aborts_shell() {
    // #2763 mirror for a shell-context null command: a funsub `exit`
    // in a numbered-fd body aborts the enclosing shell with N.
    let (stdout, _, code) = run_script(
        "fd3-funsub-aborts",
        "echo \"C1-before\"\n3<<EOF\n${ exit 42; }\nEOF\necho \"C2-after status=$?\"\necho \"C3-next-ran\"\n",
    );
    assert_eq!(stdout, "C1-before\n");
    assert_eq!(code, Some(42));
}

#[test]
fn fd0_funsub_exit_is_command_status() {
    // #2763 mirror for a fork-context null command: a funsub `exit`
    // in an fd-0 body kills the child during do_redirections, so the
    // command never runs and N is its status; the script continues.
    let (stdout, _, code) = run_script(
        "fd0-funsub-status",
        "echo \"D1-before\"\n<<EOF\n${ exit 42; }\nEOF\necho \"D2-after status=$?\"\necho \"D3-next-ran\"\n",
    );
    assert_eq!(stdout, "D1-before\nD2-after status=42\nD3-next-ran\n");
    assert_eq!(code, Some(0));
}
