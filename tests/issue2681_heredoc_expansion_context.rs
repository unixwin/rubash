//! rubash#2681 — heredoc / here-string bodies were expanded in the
//! PARENT shell for every command, so expansion side effects
//! (`${var:=word}`, arithmetic, substitutions) leaked out of
//! commands GNU forks for. GNU expands a body inside
//! do_redirections (redir.c:348 heredoc_expand): for a disk command
//! that is in the forked child (execute_cmd.c:5522
//! execute_disk_command), for a builtin/function in the current
//! shell (execute_cmd.c:5606 execute_builtin_or_function), and for
//! a null command under execute_null_command's force-fork rule.
//! The expansion must therefore run in the command's execution
//! context: effects persist exactly for builtin/function targets.
//!
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::process::Command;

fn run_script(name: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-issue2681-{name}-{}", std::process::id()));
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
fn external_command_discards_body_assignment() {
    let (stdout, _, code) = run_script(
        "external-discards",
        "unset x\ncat <<EOT >/dev/null\n${x:=v}\nEOT\necho \"external: ${x-unset}\"\n",
    );
    assert_eq!(stdout, "external: unset\n");
    assert_eq!(code, Some(0));
}

#[test]
fn colon_builtin_keeps_body_assignment() {
    let (stdout, _, code) = run_script(
        "colon-keeps",
        "unset b\n: <<EOT\n${b:=B}\nEOT\necho \"colon: ${b-unset}\"\n",
    );
    assert_eq!(stdout, "colon: B\n");
    assert_eq!(code, Some(0));
}

#[test]
fn read_builtin_keeps_body_assignment() {
    let (stdout, _, code) = run_script(
        "read-keeps",
        "unset c\nread x <<EOT\n${c:=C}\nEOT\necho \"read-builtin: ${c-unset} x=$x\"\n",
    );
    assert_eq!(stdout, "read-builtin: C x=C\n");
    assert_eq!(code, Some(0));
}

#[test]
fn exec_numbered_heredoc_arithmetic_persists() {
    let (stdout, _, code) = run_script(
        "exec-arith",
        "n=10\nexec 3<<EOT\n$((n+=1))\nEOT\necho \"after exec heredoc arith: n=$n\"\nexec 3<&-\n",
    );
    assert_eq!(stdout, "after exec heredoc arith: n=11\n");
    assert_eq!(code, Some(0));
}

#[test]
fn external_arithmetic_does_not_persist() {
    let (stdout, _, code) = run_script(
        "external-arith",
        "n=10\ncat <<EOT >/dev/null\n$((n+=100))\nEOT\necho \"after external: n=$n\"\n",
    );
    assert_eq!(stdout, "after external: n=10\n");
    assert_eq!(code, Some(0));
}

#[test]
fn env_wrapper_discards() {
    // GNU has no env builtin: `env cat` is a disk command, so the
    // body expands in env's process and its effects die there.
    let (stdout, _, code) = run_script(
        "env-wrapper",
        "unset e1\nenv cat <<EOT >/dev/null\n${e1:=E1}\nEOT\necho \"env-wrap: ${e1-unset}\"\n",
    );
    assert_eq!(stdout, "env-wrap: unset\n");
    assert_eq!(code, Some(0));
}

#[test]
fn command_wrapper_discards_for_external_target() {
    let (stdout, _, code) = run_script(
        "command-wrapper",
        "unset e2\ncommand cat <<EOT >/dev/null\n${e2:=E2}\nEOT\necho \"command-wrap: ${e2-unset}\"\n",
    );
    assert_eq!(stdout, "command-wrap: unset\n");
    assert_eq!(code, Some(0));
}

#[test]
fn command_wrapper_keeps_for_builtin_target() {
    // `command :` peels to the builtin (execute_cmd.c
    // check_command_builtin), so the body expands in the shell.
    let (stdout, _, code) = run_script(
        "command-builtin-target",
        "unset e3\ncommand : <<EOT\n${e3:=E3}\nEOT\necho \"command-colon: ${e3-unset}\"\n",
    );
    assert_eq!(stdout, "command-colon: E3\n");
    assert_eq!(code, Some(0));
}

#[test]
fn command_v_keeps_body_assignment() {
    // `command -v` runs no command at all: the `command` builtin
    // itself executes, with the command's redirections, in the shell.
    let (stdout, _, code) = run_script(
        "command-v",
        "unset v\ncommand -v cat <<EOT >/dev/null\n${v:=x}\nEOT\necho \"command-v: ${v-unset}\"\n",
    );
    assert_eq!(stdout, "command-v: x\n");
    assert_eq!(code, Some(0));
}

#[test]
fn function_calling_external_discards() {
    let (stdout, _, code) = run_script(
        "function-external",
        "unset v\nf() {\ncat <<EOT >/dev/null\n${v:=V}\nEOT\n}\nf\necho \"func-external: ${v-unset}\"\n",
    );
    assert_eq!(stdout, "func-external: unset\n");
    assert_eq!(code, Some(0));
}

#[test]
fn loop_iterations_do_not_poison_each_other() {
    // B2 hd63: each iteration's `cat` child starts from the loop's
    // environment — the previous iteration's binding died with its
    // child.
    let (stdout, _, code) = run_script(
        "loop-poison",
        "unset c\nfor i in 1 2; do\ncat <<EOT\niter $i ${c:=set$i}\nEOT\ndone\necho \"after loop c=${c-unset}\"\n",
    );
    assert_eq!(stdout, "iter 1 set1\niter 2 set2\nafter loop c=unset\n");
    assert_eq!(code, Some(0));
}

#[test]
fn earlier_body_effects_visible_to_later_body_in_child() {
    // B2 hd68: bodies of one command expand in redirect order in
    // the one child — the overridden body's binding is visible when
    // the surviving body expands, and nothing reaches the parent.
    let (stdout, _, code) = run_script(
        "inter-body",
        "unset a\ncat <<C <<D\nC-body second ${a:=fromD}\nC\nD-body first-seen ${a-unset}\nD\necho \"after: a=${a-unset}\"\n",
    );
    assert_eq!(stdout, "D-body first-seen fromD\nafter: a=unset\n");
    assert_eq!(code, Some(0));
}

#[test]
fn overridden_body_error_fires_and_is_contained() {
    // B2 hd70: the overridden body's `${var:?}` still fires (all
    // bodies expand), the command fails with status 1, and the
    // script continues.
    let (stdout, stderr, code) = run_script(
        "overridden-error",
        "cat <<A <<B\n${UNDEF_Z:?overridden-body-error}\nA\nreal-body\nB\necho \"rc=$? survived\"\n",
    );
    assert_eq!(stdout, "rc=1 survived\n");
    assert!(
        stderr.contains("UNDEF_Z: overridden-body-error"),
        "{stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn overridden_body_comsub_side_effect_fires() {
    // B2 hd71: a command substitution in the overridden body still
    // executes.
    let (stdout, _, code) = run_script(
        "overridden-side-effect",
        "cat <<A <<B\n$(touch overridden_ran.txt)\nA\nreal-body\nB\nls overridden_ran.txt 2>/dev/null || echo \"no-side-effect-file\"\n",
    );
    assert_eq!(stdout, "real-body\noverridden_ran.txt\n");
    assert_eq!(code, Some(0));
}

#[test]
fn earlier_body_assignment_discarded_when_later_body_errors() {
    let (stdout, stderr, code) = run_script(
        "error-discards-earlier",
        "unset keep\ncat <<A <<B\n${keep:=1}\nA\n${NEVER_SET:?boom}\nB\necho \"rc=$? keep=${keep-unset} survived\"\n",
    );
    assert_eq!(stdout, "rc=1 keep=unset survived\n");
    assert!(stderr.contains("NEVER_SET: boom"), "{stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn builtin_body_error_is_fatal_to_the_script() {
    // The same shape against a special builtin: the expansion error
    // lands in the main shell and the noninteractive shell exits.
    let (stdout, stderr, code) = run_script(
        "builtin-error-fatal",
        "unset keep\n: <<A <<B\n${keep:=1}\nA\n${NEVER_SET2:?boom}\nB\necho \"rc=$? keep=${keep-unset} survived\"\n",
    );
    assert_eq!(stdout, "");
    assert!(stderr.contains("NEVER_SET2: boom"), "{stderr}");
    assert_eq!(code, Some(1));
}

#[test]
fn comsub_wrapped_external_discards() {
    // B2 hd21: the command substitution is itself a fork; the cat
    // inside it forks again — the binding reaches neither.
    let (stdout, _, code) = run_script(
        "comsub-wrapped",
        "unset z\nout=$(cat <<EOT\n${z:=ZED}\nEOT\n)\necho \"out=$out z=${z-unset}\"\n",
    );
    assert_eq!(stdout, "out=ZED z=unset\n");
    assert_eq!(code, Some(0));
}

#[test]
fn background_command_discards() {
    // The background cat still prints the expanded body (`v`); the
    // binding itself dies with the async subshell.
    let (stdout, _, code) = run_script(
        "background",
        "unset p\ncat <<EOT &\n${p:=v}\nEOT\nwait\necho \"background: ${p-unset}\"\n",
    );
    assert_eq!(stdout, "v\nbackground: unset\n");
    assert_eq!(code, Some(0));
}

#[test]
fn time_wrapper_discards() {
    let (stdout, _, code) = run_script(
        "time-wrapper",
        "unset t\ntime cat <<EOT >/dev/null\n${t:=v}\nEOT\necho \"time-wrap: ${t-unset}\"\n",
    );
    assert_eq!(stdout, "time-wrap: unset\n");
    assert_eq!(code, Some(0));
}

#[test]
fn eval_external_discards() {
    let (stdout, _, code) = run_script(
        "eval-external",
        "unset q\neval 'cat <<EOT >/dev/null\n${q:=v}\nEOT\n'\necho \"eval-external: ${q-unset}\"\n",
    );
    assert_eq!(stdout, "eval-external: unset\n");
    assert_eq!(code, Some(0));
}

#[test]
fn disabled_builtin_dispatches_to_disk_and_discards() {
    let (stdout, _, code) = run_script(
        "disabled-builtin",
        "unset v\nenable -n echo 2>/dev/null\necho hi <<EOT >/dev/null\n${v:=x}\nEOT\necho \"disabled-builtin: v=${v-unset}\"\n",
    );
    assert_eq!(stdout, "disabled-builtin: v=unset\n");
    assert_eq!(code, Some(0));
}

#[test]
fn command_cat_error_is_contained() {
    // `command cat` forks, so its fatal heredoc error is contained
    // (status 1, script continues) — classification follows the
    // effective command word, not the literal first word.
    let (stdout, stderr, code) = run_script(
        "command-cat-contained",
        "command cat <<EOT\n${UNDEF_W:?contained-error}\nEOT\necho \"rc=$? alive-after-command-cat\"\n",
    );
    assert_eq!(stdout, "rc=1 alive-after-command-cat\n");
    assert!(stderr.contains("UNDEF_W: contained-error"), "{stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn prefix_assignment_not_visible_in_body_and_not_kept() {
    // GNU probe (5.3.20): a temporary prefix assignment is NOT
    // visible to the command's own heredoc body — the child's
    // do_redirections expansion consults the ordinary variable
    // table, and the prefix assignments are merged into the exec
    // environment only afterwards (execute_cmd.c
    // maybe_make_export_env, just before execve). Nor do they
    // persist after the command.
    let (stdout, _, code) = run_script(
        "prefix-assignment",
        "y2=inner cat <<EOT\nprefix sees $y2\nEOT\necho \"after: y2=${y2-unset}\"\n",
    );
    assert_eq!(stdout, "prefix sees \nafter: y2=unset\n");
    assert_eq!(code, Some(0));
}

#[test]
fn pipeline_stage_discards() {
    let (stdout, _, code) = run_script(
        "pipeline-stage",
        "unset p\ncat <<EOT | grep .\n${p:=v}\nEOT\necho \"pipeline: ${p-unset}\"\n",
    );
    assert_eq!(stdout, "v\npipeline: unset\n");
    assert_eq!(code, Some(0));
}

#[test]
fn vanished_words_null_command_error_is_contained() {
    // The words all expand away: a null command whose fd-0 redirect
    // forces GNU's fork (execute_null_command), so the fatal body
    // error is contained.
    let (stdout, stderr, code) = run_script(
        "vanished-null",
        "EMPTY=\n$EMPTY <<EOT\n${UNDEF_V:?boom}\nEOT\necho \"rc=$? alive\"\n",
    );
    assert_eq!(stdout, "rc=1 alive\n");
    assert!(stderr.contains("UNDEF_V: boom"), "{stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn set_e_external_body_error_exits() {
    let (stdout, stderr, code) = run_script(
        "set-e-external",
        "set -e\ncat <<EOT\n${UNDEF_Q:?should-fail}\nEOT\necho \"alive-external\"\n",
    );
    assert_eq!(stdout, "");
    assert!(stderr.contains("UNDEF_Q: should-fail"), "{stderr}");
    assert_ne!(code, Some(0));
}

#[test]
fn set_e_builtin_body_error_exits() {
    let (stdout, stderr, code) = run_script(
        "set-e-builtin",
        "set -e\n: <<EOT\n${UNDEF_R:?should-fail}\nEOT\necho \"alive-builtin\"\n",
    );
    assert_eq!(stdout, "");
    assert!(stderr.contains("UNDEF_R: should-fail"), "{stderr}");
    assert_ne!(code, Some(0));
}

#[test]
fn herestring_external_discards() {
    let (stdout, _, code) = run_script(
        "herestring-external",
        "unset h\ncat <<< \"${h:=v}\" >/dev/null\necho \"herestring-external: ${h-unset}\"\n",
    );
    assert_eq!(stdout, "herestring-external: unset\n");
    assert_eq!(code, Some(0));
}

#[test]
fn word_expansion_assignment_still_persists() {
    // Control: a `:=` in the command's WORDS expands in the shell
    // (word expansion precedes the fork) and must keep persisting —
    // only the heredoc bodies moved into the child context.
    let (stdout, _, code) = run_script(
        "word-assign-control",
        "unset w\ncat ${w:=word} <<EOT >/dev/null\nbody\nEOT\necho \"word-assign: ${w-unset}\"\n",
    );
    assert_eq!(stdout, "word-assign: word\n");
    assert_eq!(code, Some(0));
}
