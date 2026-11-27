//! rubash#2574 — `resolve_compspec` looked the command word up
//! verbatim only: a compspec registered for `git` governed neither
//! `/usr/bin/git …` (the basename retry) nor `g …` for
//! `alias g=git` under `shopt -s progcomp_alias` (the alias retry).
//!
//! GNU ownership: pcomplete.c programmable_completions tries, in
//! order, (1) the compspec for the command as typed (ocmd), (2) the
//! compspec for ocmd's basename when ocmd contains a `/` (the spec
//! still runs with cmd = ocmd verbatim), (3) the -D default compspec,
//! and (4) — only when progcomp_alias is set and ocmd is an alias —
//! a rebuilt line whose command word is the alias value's first word
//! (bashline.c skip_to_delim over `"()<>&;| \t\n"`), restarting the
//! lookup; the lastcs guard keeps a spec from running twice and an
//! alias cycle from spinning (ported as a visited set).

use rubash::executor::Executor;
use rubash::lexer::tokenize;
use rubash::parser::parse;

fn run_script(executor: &mut Executor, script: &str) {
    let tokens = tokenize(script);
    let ast = parse(&tokens);
    let result = executor.execute_ast(&ast);
    assert!(result.is_ok(), "script failed: {script:?} -> {result:?}");
}

#[test]
fn basename_retry_finds_the_spec() {
    let mut executor = Executor::new();
    run_script(&mut executor, "complete -W 'zzzbase' git");
    let line = "/usr/bin/git zz";
    assert_eq!(
        executor.complete_line(line, line.len()),
        vec!["zzzbase".to_string()],
        "the git spec governs /usr/bin/git via the basename retry"
    );
    // A different basename does not borrow the spec.
    let other = "/usr/bin/hg zz";
    assert!(
        !executor
            .complete_line(other, other.len())
            .contains(&"zzzbase".to_string()),
        "no cross-command leakage through the basename retry"
    );
}

#[test]
fn alias_retry_requires_progcomp_alias() {
    let mut off = Executor::new();
    run_script(&mut off, "alias g=git; complete -W 'zzzalias' git");
    let line = "g zz";
    assert!(
        !off.complete_line(line, line.len())
            .contains(&"zzzalias".to_string()),
        "progcomp_alias off (the default): the alias retry must not fire"
    );

    let mut on = Executor::new();
    run_script(
        &mut on,
        "shopt -s progcomp_alias; alias g=git; complete -W 'zzzalias' git",
    );
    assert_eq!(
        on.complete_line(line, line.len()),
        vec!["zzzalias".to_string()],
        "progcomp_alias on: g resolves through the alias to git's spec"
    );
}

#[test]
fn alias_retry_uses_the_alias_values_first_word() {
    let mut executor = Executor::new();
    run_script(
        &mut executor,
        "shopt -s progcomp_alias; alias gg='git status'; complete -W 'zzzalias' git",
    );
    let line = "gg zz";
    assert_eq!(
        executor.complete_line(line, line.len()),
        vec!["zzzalias".to_string()],
        "the hop is the alias value's first word (git), not the whole value"
    );
}

#[test]
fn default_spec_precedes_the_alias_retry() {
    let mut executor = Executor::new();
    run_script(
        &mut executor,
        "shopt -s progcomp_alias; alias g=git; complete -W 'zzzgit' git; complete -D -W 'zzzdefault'",
    );
    let line = "g zz";
    assert_eq!(
        executor.complete_line(line, line.len()),
        vec!["zzzdefault".to_string()],
        "GNU tries -D before the alias retry"
    );
}

#[test]
fn alias_cycle_terminates() {
    let mut executor = Executor::new();
    run_script(
        &mut executor,
        "shopt -s progcomp_alias; alias a=b; alias b=a",
    );
    // No spec anywhere: the chain must end (visited set), not spin.
    let line = "a zz";
    let _ = executor.complete_line(line, line.len());
}
