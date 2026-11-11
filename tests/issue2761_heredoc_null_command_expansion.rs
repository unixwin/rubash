//! rubash#2761 — heredoc / here-string bodies on raw-empty null
//! commands were never expanded: the empty-words arm
//! (`cmd.words.is_empty()`, command_execute.rs) routed straight to
//! `execute_empty_words_command`, bypassing the heredoc pre-expansion
//! funnel entirely, so a command substitution in the body never
//! executed and `:=` never bound.
//!
//! GNU expands the body at `do_redirections` time
//! (execute_cmd.c:4203-4278 execute_null_command, called from
//! execute_cmd.c:4626-4640 when the expanded words are empty;
//! redir.c:237 do_redirections walks the list in order;
//! redir.c:1065-1072 do_redirection_internal hands r_reading_until /
//! r_reading_string to here_document_to_fd, redir.c:423-434, which
//! expands first via heredoc_expand, redir.c:348). The force-fork
//! rule (execute_cmd.c:4210-4216: any redirect targeting fd 0, or a
//! REDIR_VARASSIGN redirect) decides the context: an fd-0 null
//! command's body expands in a forked child (side effects execute,
//! nothing persists), while a numbered-fd form (`3<<EOF`) expands
//! in the shell (`:=` binds persist).
//!
//! Expectations verified against GNU bash 5.3.20 (script files);
//! the fd-0 / numbered-fd split is stated in the issue itself.

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
fn null_heredoc_fd0_comsub_side_effect_executes() {
    // The issue's headline case: `<<EOF` alone, body holds a
    // command substitution — GNU runs it at do_redirections time.
    let (stdout, _, code) = run_script(
        "fd0-comsub",
        "<<EOF\n$(touch side_effect.txt)\nEOF\nls side_effect.txt 2>/dev/null || echo \"no-side-effect-file\"\n",
    );
    assert_eq!(stdout, "side_effect.txt\n");
    assert_eq!(code, Some(0));
}

#[test]
fn null_heredoc_fd0_assign_does_not_persist() {
    // fd-0 null command force-forks (execute_cmd.c:4210-4216), so
    // the body's `:=` binds in the child and dies with it.
    let (stdout, _, code) = run_script(
        "fd0-assign-discarded",
        "unset v\n<<EOF\n${v:=bound}\nEOF\necho \"v=${v-unset}\"\n",
    );
    assert_eq!(stdout, "v=unset\n");
    assert_eq!(code, Some(0));
}

#[test]
fn null_heredoc_fd0_arithmetic_discarded() {
    let (stdout, _, code) = run_script(
        "fd0-arith-discarded",
        "n=10\n<<EOF\n$((n+=1))\nEOF\necho \"n=$n\"\n",
    );
    assert_eq!(stdout, "n=10\n");
    assert_eq!(code, Some(0));
}

#[test]
fn numbered_heredoc_assign_persists() {
    // `3<<EOF` does not force the fork, so the body expands in the
    // shell and `:=` binds persist.
    let (stdout, _, code) = run_script(
        "numbered-assign",
        "unset v\n3<<EOF\n${v:=bound}\nEOF\necho \"v=${v-unset}\"\n",
    );
    assert_eq!(stdout, "v=bound\n");
    assert_eq!(code, Some(0));
}

#[test]
fn numbered_heredoc_arithmetic_persists() {
    let (stdout, _, code) = run_script(
        "numbered-arith",
        "n=10\n3<<EOF\n$((n+=1))\nEOF\necho \"n=$n\"\n",
    );
    assert_eq!(stdout, "n=11\n");
    assert_eq!(code, Some(0));
}

#[test]
fn numbered_heredoc_comsub_side_effect_executes() {
    let (stdout, _, code) = run_script(
        "numbered-comsub",
        "3<<EOF\n$(touch numbered_ran.txt)\nEOF\nls numbered_ran.txt 2>/dev/null || echo \"no-side-effect-file\"\n",
    );
    assert_eq!(stdout, "numbered_ran.txt\n");
    assert_eq!(code, Some(0));
}

#[test]
fn prefix_assignment_visible_in_numbered_body() {
    // GNU binds the null command's prefix assignments during word
    // expansion, before do_redirections expands the body — the body
    // sees the bound value.
    let (stdout, _, code) = run_script(
        "prefix-visible",
        "unset y\nx=hello 3<<EOF\n${y:=$x}\nEOF\necho \"y=${y-unset}\"\n",
    );
    assert_eq!(stdout, "y=hello\n");
    assert_eq!(code, Some(0));
}

#[test]
fn vanished_words_heredoc_comsub_executes() {
    // Words that expand to nothing take the same null-command path
    // (execute_cmd.c:4626-4640): the body still expands.
    let (stdout, _, code) = run_script(
        "vanished-comsub",
        "EMPTY=\n$EMPTY <<EOF\n$(touch vanished_ran.txt)\nEOF\nls vanished_ran.txt 2>/dev/null || echo \"no-side-effect-file\"\n",
    );
    assert_eq!(stdout, "vanished_ran.txt\n");
    assert_eq!(code, Some(0));
}

#[test]
fn vanished_words_numbered_heredoc_assign_persists() {
    let (stdout, _, code) = run_script(
        "vanished-numbered-assign",
        "EMPTY=\nunset v\n$EMPTY 3<<EOF\n${v:=bound}\nEOF\necho \"v=${v-unset}\"\n",
    );
    assert_eq!(stdout, "v=bound\n");
    assert_eq!(code, Some(0));
}

#[test]
fn two_heredocs_both_expand_in_order() {
    // GNU do_redirections walks the list in document order: both
    // bodies expand, earlier effects visible to later bodies.
    let (stdout, _, code) = run_script(
        "two-bodies-order",
        "<<A <<B\n$(echo first >> order.txt)\nA\n$(echo second >> order.txt)\nB\ncat order.txt\n",
    );
    assert_eq!(stdout, "first\nsecond\n");
    assert_eq!(code, Some(0));
}

#[test]
fn numbered_bodies_expand_in_order() {
    // Same ordering, in the shell context: the second body's
    // expansion sees the first body's binding.
    let (stdout, _, code) = run_script(
        "numbered-order",
        "unset b\n3<<A 4<<B\n${a:=fromA}\nA\n${b:=$a}\nB\necho \"b=${b-unset}\"\n",
    );
    assert_eq!(stdout, "b=fromA\n");
    assert_eq!(code, Some(0));
}

#[test]
fn quoted_heredoc_body_not_expanded() {
    // Control: a quoted delimiter suppresses body expansion
    // (redir.c:362 heredoc_expand's W_QUOTED branch) — the
    // substitution must NOT run.
    let (stdout, _, code) = run_script(
        "quoted-no-expand",
        "<<'EOF'\n$(touch quoted_ran.txt)\nEOF\nls quoted_ran.txt 2>/dev/null || echo \"no-side-effect-file\"\n",
    );
    assert_eq!(stdout, "no-side-effect-file\n");
    assert_eq!(code, Some(0));
}

#[test]
fn null_herestring_comsub_executes() {
    // A bare here-string is the same null-command shape
    // (r_reading_string, redir.c:1067): its word expands at
    // do_redirections time.
    let (stdout, _, code) = run_script(
        "herestring-comsub",
        "<<<\"$(touch hs_ran.txt)\"\nls hs_ran.txt 2>/dev/null || echo \"no-side-effect-file\"\n",
    );
    assert_eq!(stdout, "hs_ran.txt\n");
    assert_eq!(code, Some(0));
}

#[test]
fn colon_builtin_body_assignment_keeps() {
    // Control (already covered by issue2681): `:` is a builtin, so
    // its body expands in the shell and the binding persists — the
    // contrast the null-command fork rule is measured against.
    let (stdout, _, code) = run_script(
        "colon-control",
        "unset b\n: <<EOF\n${b:=B}\nEOF\necho \"colon: ${b-unset}\"\n",
    );
    assert_eq!(stdout, "colon: B\n");
    assert_eq!(code, Some(0));
}

#[test]
fn arith_error_fd0_status_1_survives() {
    // An arithmetic error in the body is the funnel's latched
    // failure class: the fd-0 child fails, the parent reports
    // status 1 and the script continues (GNU 5.3.20 oracle P12).
    let (stdout, _, code) = run_script(
        "arith-error-fd0",
        "<<EOF\n$((1/0))\nEOF\necho \"rc=$? survived\"\n",
    );
    assert_eq!(stdout, "rc=1 survived\n");
    assert_eq!(code, Some(0));
}

#[test]
fn arith_error_numbered_status_1_survives() {
    // Same, in the shell context (GNU 5.3.20 oracle P13).
    let (stdout, _, code) = run_script(
        "arith-error-numbered",
        "3<<EOF\n$((1/0))\nEOF\necho \"rc=$? survived\"\n",
    );
    assert_eq!(stdout, "rc=1 survived\n");
    assert_eq!(code, Some(0));
}

#[test]
fn fd0_fatal_body_error_contained() {
    // A fatal `${var:?}` in an fd-0 null body is contained by the
    // force-fork: status 1, script continues (GNU 5.3.20 oracle P17).
    let (stdout, stderr, code) = run_script(
        "fd0-fatal-contained",
        "<<EOF\n${UNDEF_Z:?boom}\nEOF\necho \"rc=$? alive\"\n",
    );
    assert_eq!(stdout, "rc=1 alive\n");
    assert!(stderr.contains("UNDEF_Z: boom"), "{stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn post_alias_empty_heredoc_comsub_executes() {
    // The third empty-words arm: alias expansion empties the words
    // (execute_cmd.c:4626-4640), and the body still expands
    // (GNU 5.3.20 oracle P18).
    let (stdout, _, code) = run_script(
        "post-alias-comsub",
        "shopt -s expand_aliases\nalias z=''\nz <<EOF\n$(touch alias_ran.txt)\nEOF\nls alias_ran.txt 2>/dev/null || echo \"no-side-effect-file\"\n",
    );
    assert_eq!(stdout, "alias_ran.txt\n");
    assert_eq!(code, Some(0));
}
