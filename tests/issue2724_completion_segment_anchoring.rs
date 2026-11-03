//! Issue rubash#2724 regression (deep-audit B5-10): compspec
//! resolution anchored on the line's FIRST word — `split_for_completion`
//! split the whole line prefix, so a compspec never governed a command
//! after `;`, `|`, `&&`, a newline, `(` or an assignment prefix, and the
//! first command's spec governed every later segment.
//!
//! GNU ownership:
//! - bashline.c:1465 `find_cmd_start` / `find_cmd_end` over
//!   COMMAND_SEPARATORS (bashline.c:1420): the command segment
//!   containing the point is found by scanning with skip_to_delim
//!   (subst.c:2199), which skips closed substitutions and quoted spans
//!   as units — an unterminated `$(` does NOT open a new segment.
//! - bashline.c:1718-1726: assignment words before the command name
//!   are skipped; pcomplete.c:1323 builds the hook's word list from the
//!   name on, and pcomplete.c:931 binds COMP_LINE / COMP_POINT
//!   segment-relative.
//!
//! Oracle: GNU bash 5.3.20 on a real pty (`complete -F` echoing a
//! marker): `true; mycmd sp`, `true && mycmd sp`, `(mycmd sp` and
//! `X=1 mycmd sp` all fire mycmd's spec with COMP_WORDS [mycmd sp]
//! (assignments excluded); `mycmd sp | other o` fires other's spec;
//! `echo $(mycmd sp` does NOT fire mycmd's spec (the outer command
//! governs) — that face was already conformant and must stay so.

use rubash::executor::Executor;
use rubash::lexer::tokenize;
use rubash::parser::parse;

fn run_script(executor: &mut Executor, script: &str) {
    let tokens = tokenize(script);
    let ast = parse(&tokens);
    let result = executor.execute_ast(&ast);
    assert!(result.is_ok(), "script failed: {script:?} -> {result:?}");
}

fn executor_with_specs() -> Executor {
    let mut executor = Executor::new();
    run_script(
        &mut executor,
        r#"_seg() { COMPREPLY=(seghit); }
           _oth() { COMPREPLY=(otherhit); }
           _out() { COMPREPLY=(outerhit); }
           _dfl() { COMPREPLY=(defaulthit); }
           complete -F _seg mycmd
           complete -F _oth other
           complete -F _out echo
           complete -D -F _dfl"#,
    );
    executor
}

fn complete_end(executor: &mut Executor, line: &str) -> Vec<String> {
    executor.complete_line(line, line.len())
}

#[test]
fn issue_2724_spec_governs_after_semicolon() {
    let mut executor = executor_with_specs();
    assert_eq!(
        complete_end(&mut executor, "true; mycmd sp"),
        vec!["seghit".to_string()]
    );
}

#[test]
fn issue_2724_spec_governs_after_and_list() {
    let mut executor = executor_with_specs();
    assert_eq!(
        complete_end(&mut executor, "true && mycmd sp"),
        vec!["seghit".to_string()]
    );
    assert_eq!(
        complete_end(&mut executor, "false || mycmd sp"),
        vec!["seghit".to_string()]
    );
}

#[test]
fn issue_2724_spec_governs_after_pipe_and_newline() {
    let mut executor = executor_with_specs();
    assert_eq!(
        complete_end(&mut executor, "true | mycmd sp"),
        vec!["seghit".to_string()]
    );
    assert_eq!(
        complete_end(&mut executor, "true\nmycmd sp"),
        vec!["seghit".to_string()]
    );
}

#[test]
fn issue_2724_second_segment_uses_its_own_spec() {
    let mut executor = executor_with_specs();
    assert_eq!(
        complete_end(&mut executor, "mycmd sp | other o"),
        vec!["otherhit".to_string()]
    );
}

#[test]
fn issue_2724_default_spec_applies_per_segment() {
    // No spec is registered for `zz`; the -D spec must govern its
    // segment even though the first segment's command HAS a spec.
    let mut executor = executor_with_specs();
    assert_eq!(
        complete_end(&mut executor, "mycmd sp | zz o"),
        vec!["defaulthit".to_string()]
    );
}

#[test]
fn issue_2724_spec_governs_after_assignment_prefix() {
    let mut executor = executor_with_specs();
    assert_eq!(
        complete_end(&mut executor, "X=1 mycmd sp"),
        vec!["seghit".to_string()]
    );
}

#[test]
fn issue_2724_spec_governs_inside_subshell_paren() {
    let mut executor = executor_with_specs();
    assert_eq!(
        complete_end(&mut executor, "(mycmd sp"),
        vec!["seghit".to_string()]
    );
}

#[test]
fn issue_2724_unterminated_substitution_stays_with_outer_command() {
    // GNU-conformant before and after: the segmenter must not descend
    // into an unterminated `$(` — echo's spec governs, mycmd's does
    // not fire.
    let mut executor = executor_with_specs();
    assert_eq!(
        complete_end(&mut executor, "echo $(mycmd sp"),
        vec!["outerhit".to_string()]
    );
    assert_eq!(
        complete_end(&mut executor, "echo `mycmd sp"),
        vec!["outerhit".to_string()]
    );
}

#[test]
fn issue_2724_comp_line_and_point_are_segment_relative() {
    let mut executor = Executor::new();
    run_script(
        &mut executor,
        r#"_rec() { COMPREPLY=("${COMP_LINE}|${COMP_POINT}|${COMP_WORDS[*]}|${COMP_CWORD}"); }
           complete -F _rec mycmd"#,
    );
    assert_eq!(
        complete_end(&mut executor, "true; mycmd sp"),
        vec!["mycmd sp|8|mycmd sp|1".to_string()]
    );
}
