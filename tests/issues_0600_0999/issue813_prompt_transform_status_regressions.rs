//! Issue rubash#813 item 1: the `${var@P}` prompt-transform channel must not
//! leak command-substitution status into the reader's `$?`.
//!
//! GNU law (third_party/bash parse.y, vendored 5.3): decode_prompt_string
//! snapshots `last_command_exit_value`/`last_command_subst_status` (and the
//! pid/lastarg pair) before the promptvars pass (parse.y:6684-6689) and
//! restores them after (parse.y:6693-6697); subst.c string_transform 'P'
//! routes through the same decode_prompt_string, so the `${var@P}` operator
//! path is covered by the same restore. rubash#604 fixed the interactive
//! render channel (`expand_prompt_string_mut` snapshots/restores the exit
//! code and PIPESTATUS); the transform channel still published the embedded
//! `$( )` body's reaped status through the comsub-status cell that the
//! assignment boundary reads for the POSIX substitution exit rule, so
//! `PS1='x$(false)y'; p=${PS1@P}` left `$?` at 1.
//!
//! GNU differential evidence (WSL GNU bash 5.3.0, stdin probes 2026-10-19):
//!   true;  PS1='x$(false)y'; p=${PS1@P}  -> rc 0, p=[xy]
//!   false; q=${PS1@P}                    -> rc 0 (assignment exit rule
//!                                           reads no substitution status)
//!   q2=$(true)                           -> rc 0 (rule itself intact)

use rubash::executor::Executor;
use rubash::lexer::tokenize;
use rubash::parser::parse;

fn run(exec: &mut Executor, src: &str) {
    let ast = parse(&tokenize(src));
    exec.execute_ast(&ast).ok();
}

/// The ticket's repro: an embedded `$(false)` in the transformed value must
/// not move the reader's `$?` (GNU restores the status words around the
/// pass, parse.y:6695-6697).
#[test]
fn prompt_transform_leaves_reader_exit_code_alone() {
    let mut exec = Executor::new();
    run(&mut exec, "true; PS1='x$(false)y'; p=${PS1@P}");
    assert_eq!(exec.last_exit_code(), 0, "render status leaked into $?");
    assert_eq!(exec.get_env("p"), Some("xy"));
}

/// The assignment exit rule itself is untouched: a REAL substitution in an
/// assignment RHS still publishes its status (`q=$(false)` -> rc 1), while
/// the @P transform — where GNU restores the status — keeps rc 0 even after
/// a failing command (`false; q=${PS1@P}` -> rc 0, WSL probe).
#[test]
fn prompt_transform_status_isolated_from_assignment_rule() {
    let mut exec = Executor::new();

    run(&mut exec, "false; q=${PS1@P}");
    assert_eq!(
        exec.last_exit_code(),
        0,
        "no substitution status may survive the @P pass (GNU rc 0)"
    );

    // Control: the substitution exit rule on ordinary RHS values is intact.
    run(&mut exec, "q2=$(false)");
    assert_eq!(exec.last_exit_code(), 1, "v=$(false) must still be rc 1");
    run(&mut exec, "q3=$(true)");
    assert_eq!(exec.last_exit_code(), 0);
}

/// Nested transforms and repeat renders: the pre-pass cell value is what the
/// pass must restore, so a publishable status raised BEFORE the transform is
/// not clobbered by it, and consecutive transforms do not accumulate state.
#[test]
fn prompt_transform_restore_is_value_preserving() {
    let mut exec = Executor::new();

    // A substitution status published by the enclosing word expansion right
    // before the transform must survive the transform's restore bracket.
    run(&mut exec, "PS1='r$(true)'; w=$(false)${PS1@P}");
    // The word's own $(false) governs the null-command status (GNU prints 1
    // for `w=$(false)${PS1@P}`), and the transform's $(true) must not mask
    // it — the bracket restores, it never clears a live publication.
    assert_eq!(
        exec.last_exit_code(),
        1,
        "the word's own substitution status must survive the @P bracket"
    );

    // Repeated transforms: the second render starts from the restored state.
    run(
        &mut exec,
        "true; PS1='a$(false)b'; r1=${PS1@P}; r2=${PS1@P}",
    );
    assert_eq!(exec.last_exit_code(), 0);
    assert_eq!(exec.get_env("r1"), Some("ab"));
    assert_eq!(exec.get_env("r2"), Some("ab"));
}

/// rubash#858: the word-internal comsub `$?` overlay must also be restored.
/// `PS1='$(exit 7)p '; echo ${PS1@P} $?` — GNU prints `p 0` (the `true`
/// before it), not `p 7` (the transform's embedded comsub status leaking
/// into the same command's later `$?`). dollar_question_status reads
/// word_expansion_comsub_exit first, so the bracket must cover that cell
/// too, not only last_command_substitution_status.
#[test]
fn prompt_transform_does_not_leak_into_word_level_dollar_question() {
    let mut exec = Executor::new();

    // Capture `echo ${PS1@P} $?` output via a substitution: the value the
    // reader's $? sees must be the pre-command 0, not the embedded 7.
    run(
        &mut exec,
        "PS1='$(exit 7)p '; true; captured=$(echo ${PS1@P} $?)",
    );
    assert_eq!(
        exec.get_env("captured"),
        Some("p 0"),
        "embedded comsub status must not reach the same command's $?"
    );

    // The standalone-word `$?` form and the `case ${PS1@P}` word form agree.
    run(
        &mut exec,
        "PS1='$(exit 4)zzz'; true; echo ${PS1@P}; case ${PS1@P} in zzz) hit=1;; *) hit=0;; esac",
    );
    assert_eq!(exec.get_env("hit"), Some("1"));
    assert_eq!(exec.last_exit_code(), 0);
}
