//! rubash#2571 — the `complete -I` initial-word compspec never
//! governed anything: `resolve_compspec` returned None for the
//! command word (cur_idx == 0) without consulting the registry, and
//! `Executor::has_compspec` (the host's routing gate) did not count
//! `-I` either, so a registered -I spec was dead configuration.
//!
//! GNU ownership: bashline.c:1682 loads
//! `iw_compspec = progcomp_search (INITIALWORD)` unconditionally at
//! the start of attempt_completion (when progcomp is enabled), and
//! bashline.c:1785-1792 runs programmable_completions with it for the
//! command word when no command-specific compspec matched. complete
//! -p prints `-I` registrations, so storage existed; only the lookup
//! was missing.

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
fn initial_word_spec_governs_the_command_word() {
    let mut executor = Executor::new();
    run_script(&mut executor, "complete -I -W 'zzzinit alpha'");
    // The first word completes from the -I spec's wordlist.
    let candidates = executor.complete_line("zz", 2);
    assert_eq!(candidates, vec!["zzzinit".to_string()]);
    let candidates = executor.complete_line("al", 2);
    assert_eq!(candidates, vec!["alpha".to_string()]);
}

#[test]
fn initial_word_spec_wins_the_first_word_and_default_takes_arguments() {
    let mut executor = Executor::new();
    run_script(
        &mut executor,
        "complete -I -W 'zzzinit'; complete -D -W 'zzzarg'",
    );
    assert_eq!(
        executor.complete_line("zz", 2),
        vec!["zzzinit".to_string()],
        "first word: -I governs even when -D exists"
    );
    assert_eq!(
        executor.complete_line("someprog zz", 11),
        vec!["zzzarg".to_string()],
        "argument word: -D governs; -I never leaks past the first word"
    );
}

#[test]
fn has_compspec_opens_for_initial_word_only_registration() {
    let mut executor = Executor::new();
    assert!(!executor.has_compspec("git"));
    run_script(&mut executor, "complete -I -W 'zzzinit'");
    assert!(
        executor.has_compspec("git"),
        "an -I registration alone must open the host's routing gate"
    );
}

#[test]
fn no_initial_word_spec_keeps_builtin_command_completion() {
    let mut executor = Executor::new();
    run_script(&mut executor, "complete -W 'zzzother' somecmd");
    // A spec for another command does not hijack the first word, and
    // no -I wordlist candidate appears.
    let candidates = executor.complete_line("zz", 2);
    assert!(
        !candidates.contains(&"zzzother".to_string()),
        "an unrelated command's spec must not govern the first word: {candidates:?}"
    );
}
