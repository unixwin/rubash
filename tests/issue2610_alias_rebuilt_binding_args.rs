//! rubash#2610 — after a progcomp alias hop, GNU rebuilds the line
//! with the alias value and restarts programmable completion, so a
//! governing spec's -F function / -C command bindings see the
//! REBUILT line's first word as $1 (and the rebuilt word list for
//! COMP_WORDS / $3). rubash resolved the right compspec through the
//! alias chain but kept binding the originally typed word.
//!
//! GNU ownership: pcomplete.c programmable_completions (the alias
//! arm): on a miss with progcomp_alias set, the alias value is
//! spliced into pcomp_line in place of the command word, ocmd
//! becomes the alias value's first word (skip_to_delim over
//! `"()<>&;| \t\n"`), and the whole lookup restarts — so
//! gen_progcomp_completions runs with pcomp_curcmd = the rebuilt
//! first word, and build_arg_list (pcomplete.c:1004) hands it to
//! the generator as $1.

use rubash::executor::Executor;
use rubash::lexer::tokenize;
use rubash::parser::parse;

fn run_script(executor: &mut Executor, script: &str) {
    let tokens = tokenize(script);
    let ast = parse(&tokens);
    let result = executor.execute_ast(&ast);
    assert!(result.is_ok(), "script failed: {script:?} -> {result:?}");
}

fn complete(executor: &mut Executor, line: &str) -> Vec<String> {
    executor.complete_line(line, line.len())
}

#[test]
fn alias_hop_binds_rebuilt_first_word_to_function() {
    let mut executor = Executor::new();
    run_script(
        &mut executor,
        "shopt -s progcomp_alias; alias g=git; \
         _f2610() { COMPREPLY=(\"$1\"); }; complete -F _f2610 git",
    );
    assert_eq!(
        complete(&mut executor, "g zz"),
        vec!["git".to_string()],
        "the -F function must see the rebuilt line's first word, not the typed alias"
    );
}

#[test]
fn alias_multi_hop_binds_final_first_word() {
    let mut executor = Executor::new();
    run_script(
        &mut executor,
        "shopt -s progcomp_alias; alias a2610=b2610; alias b2610=git; \
         _f2610() { COMPREPLY=(\"$1\"); }; complete -F _f2610 git",
    );
    assert_eq!(
        complete(&mut executor, "a2610 zz"),
        vec!["git".to_string()],
        "two alias hops still land on the final rebuilt first word"
    );
}

#[test]
fn alias_value_with_words_rebuilds_prev_word() {
    let mut executor = Executor::new();
    run_script(
        &mut executor,
        "shopt -s progcomp_alias; alias gg2610='git status'; \
         _f2610() { COMPREPLY=(\"$1,$3\"); }; complete -F _f2610 git",
    );
    assert_eq!(
        complete(&mut executor, "gg2610 zz"),
        vec!["git,status".to_string()],
        "the rebuilt word list is [git, status, zz]: $1=git and $3 (prev word)=status"
    );
}

#[test]
fn alias_hop_binds_rebuilt_first_word_to_command() {
    let mut executor = Executor::new();
    run_script(
        &mut executor,
        "shopt -s progcomp_alias; alias g=git; \
         complete -C 'printf \"CMD:%s\\n\"' git",
    );
    // The -C command is printf with the format "CMD:%s\n": it cycles
    // the format over ALL the arguments gen_command_matches appends
    // after it ($1 $2 $3 = rebuilt-cmdname, cur, prev = git, zz, git),
    // so GNU's verbatim candidate set is {CMD:git, CMD:zz} — verified
    // against GNU bash 5.3.20 interactively (TAB on `g zz` inserts
    // only the common prefix "CMD:", and the surviving not-found
    // error names 'CMD:', never the full 'CMD:git'). The cell's
    // assertion is the $1-derived line: CMD:git (rebuilt first word),
    // not CMD:g (the typed alias) — on the unfixed base this cell
    // yields ["CMD:g", "CMD:zz"].
    assert_eq!(
        complete(&mut executor, "g zz"),
        vec!["CMD:git".to_string(), "CMD:zz".to_string()],
        "the -C command's $1 argument is the rebuilt first word"
    );
}

#[test]
fn verbatim_command_word_binding_unchanged() {
    let mut executor = Executor::new();
    run_script(
        &mut executor,
        "_f2610() { COMPREPLY=(\"$1\"); }; complete -F _f2610 git",
    );
    assert_eq!(
        complete(&mut executor, "git zz"),
        vec!["git".to_string()],
        "control: no alias involved, the typed word is the bound word"
    );
}

#[test]
fn alias_hop_requires_progcomp_alias() {
    let mut executor = Executor::new();
    run_script(
        &mut executor,
        "alias g=git; _f2610() { COMPREPLY=(\"$1\"); }; complete -F _f2610 git",
    );
    assert!(
        !complete(&mut executor, "g zz").contains(&"git".to_string()),
        "control: progcomp_alias off — the alias chain must not run at all"
    );
}
