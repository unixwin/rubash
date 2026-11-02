//! rubash#2578 — `COMP_WORDBREAKS` was bound at startup but never read:
//! completion word splitting split on whitespace only, so `foo a=b<TAB>`
//! completed the whole `a=b` as one word and -F functions saw a
//! COMP_WORDS list GNU never produces.
//!
//! GNU ownership: bashline.c:313 `bash_completer_word_break_characters`
//! (the COMP_WORDBREAKS default) delimits the completion word, and
//! pcomplete.c bind_compfunc_variables exposes that split as COMP_WORDS /
//! COMP_CWORD. Expectations probed against the GNU bash 5.3.20 oracle
//! (interactive PTY, `complete -F` echoing the bound variables):
//!   `foo a=b`  -> COMP_WORDS=[foo a = b],  COMP_CWORD=3, cur `b`
//!   `foo a==b` -> COMP_WORDS=[foo a == b], COMP_CWORD=3, cur `b`
//!   `foo a=`   -> COMP_WORDS=[foo a =],    COMP_CWORD=2, cur `=`
//!   `foo a `   -> COMP_WORDS=[foo a ""],   COMP_CWORD=2, cur ``

use rubash::executor::Executor;
use rubash::lexer::tokenize;
use rubash::parser::parse;

fn run_script(executor: &mut Executor, script: &str) {
    let tokens = tokenize(script);
    let ast = parse(&tokens);
    let result = executor.execute_ast(&ast);
    assert!(result.is_ok(), "script failed: {script:?} -> {result:?}");
}

fn probe_words(line: &str, cursor: usize) -> String {
    let mut executor = Executor::new();
    run_script(
        &mut executor,
        r#"_f() { COMPREPLY=("${COMP_WORDS[*]}|${COMP_CWORD}|${COMP_WORDS[COMP_CWORD]}"); }
           complete -F _f foo"#,
    );
    executor.complete_line(line, cursor).join("\n")
}

#[test]
fn issue_2578_equals_breaks_word() {
    assert_eq!(probe_words("foo a=b", 7), "foo a = b|3|b");
}

#[test]
fn issue_2578_break_run_is_one_word() {
    assert_eq!(probe_words("foo a==b", 8), "foo a == b|3|b");
}

#[test]
fn issue_2578_trailing_break_is_current_word() {
    assert_eq!(probe_words("foo a=", 6), "foo a =|2|=");
}

#[test]
fn issue_2578_colon_breaks_word() {
    assert_eq!(probe_words("foo a:b", 7), "foo a : b|3|b");
}

#[test]
fn issue_2578_custom_wordbreaks_are_honored() {
    let mut executor = Executor::new();
    run_script(
        &mut executor,
        r#"COMP_WORDBREAKS=" "
           _f() { COMPREPLY=("${COMP_WORDS[*]}|${COMP_CWORD}|${COMP_WORDS[COMP_CWORD]}"); }
           complete -F _f foo"#,
    );
    assert_eq!(
        executor.complete_line("foo a=b", 7).join("\n"),
        "foo a=b|1|a=b"
    );
}

#[test]
fn issue_2578_value_after_equals_completes() {
    // The user-visible face: with a -W compspec, completing the value
    // after `opt=` must filter the wordlist by `ba`, not by `opt=ba`.
    let mut executor = Executor::new();
    run_script(&mut executor, "complete -W 'bar baz qux' foo");
    assert_eq!(
        executor.complete_line("foo opt=ba", 10),
        vec!["bar".to_string(), "baz".to_string()]
    );
}
