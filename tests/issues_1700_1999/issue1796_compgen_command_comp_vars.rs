//! Issue rubash#1796 regression: `compgen -C cmd` bound COMP_CWORD=0
//! (and an empty COMP_WORDS) for the command, where GNU binds neither.
//!
//! GNU pcomplete.c:929 bind_compfunc_variables binds COMP_WORDS and
//! COMP_CWORD only in the exported==0 (function) arm; gen_command_
//! matches (pcomplete.c:1156) calls it with exported=1 for -C, so only
//! COMP_LINE/COMP_POINT/COMP_TYPE/COMP_KEY are bound (to "", 0, 0, 0
//! in the compgen shape). A caller's own COMP_CWORD/COMP_WORDS pass
//! through untouched, and unbind_compfunc_variables (pcomplete.c:976)
//! removes all six afterwards either way.
//!
//! Oracle: GNU bash 5.3.20 — inside `compgen -C '...' foo`,
//! ${COMP_CWORD-unset} and ${COMP_WORDS-unset} both report unset;
//! with COMP_CWORD=7 preset by the script, the command sees 7.

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn compgen_command_observes_no_comp_cword_or_words() {
    // The -C command's own output is the compgen candidate list; the
    // trailing " compgen foo" is build_arg_list's appended arguments
    // landing after the echo argument (same on GNU).
    let (stdout, _, _) = rubash(
        "compgen -C 'echo \"CWORD=[${COMP_CWORD-unset}] WORDS=[${COMP_WORDS-unset}] \
         LINE=[$COMP_LINE] POINT=[$COMP_POINT] TYPE=[$COMP_TYPE] KEY=[$COMP_KEY]\"' foo",
    );
    assert_eq!(
        stdout.trim_end(),
        "CWORD=[unset] WORDS=[unset] LINE=[] POINT=[0] TYPE=[0] KEY=[0] compgen foo"
    );
}

#[test]
fn compgen_command_with_empty_word_observes_the_same() {
    let (stdout, _, _) =
        rubash("compgen -C 'echo \"CWORD=[${COMP_CWORD-unset}] WORDS=[${COMP_WORDS-unset}]\"' ''");
    assert_eq!(stdout.trim_end(), "CWORD=[unset] WORDS=[unset] compgen");
}

#[test]
fn compgen_command_passes_caller_comp_vars_through() {
    let (stdout, _, _) = rubash(
        "COMP_CWORD=7; COMP_WORDS=(a b); \
         compgen -C 'echo \"CWORD=[$COMP_CWORD] WORDS=[${COMP_WORDS[*]}]\"' foo",
    );
    assert_eq!(stdout.trim_end(), "CWORD=[7] WORDS=[a b] compgen foo");
}

#[test]
fn compgen_function_still_observes_bound_comp_vars() {
    // Control: the -F (exported==0) arm is unchanged — the function
    // sees COMP_CWORD=-1 and an empty COMP_WORDS (pcomplete.c:1073).
    let (stdout, _, _) = rubash(
        "f() { SEEN=\"CWORD=[$COMP_CWORD] WORDS=[${COMP_WORDS[*]}]\"; COMPREPLY=(x); }; \
         compgen -F f foo >/dev/null; echo \"$SEEN\"",
    );
    assert_eq!(stdout.trim_end(), "CWORD=[-1] WORDS=[]");
}

#[test]
fn comp_vars_are_unbound_after_compgen_command() {
    let (stdout, _, _) = rubash(
        "compgen -C true foo >/dev/null 2>&1; \
         echo \"LINE=${COMP_LINE+set} CWORD=${COMP_CWORD+set} WORDS=${COMP_WORDS+set}\"",
    );
    assert_eq!(stdout.trim_end(), "LINE= CWORD= WORDS=");
}
