//! Issue rubash#2726 regression (deep-audit B5-12): COMP_WORDS was
//! truncated at the cursor and empty words were dropped —
//! `split_for_completion` tokenized only `line[..cursor]`, so a hook
//! never saw the words after the cursor, the empty word after
//! trailing whitespace, or the empty word at a whitespace gap.
//!
//! GNU ownership: pcomplete.c:1238 `command_line_to_word_list` runs
//! subst.c:2616 `split_at_delims` over the whole command text
//! (pcomplete.c:1323's substring runs to the command's END): a token
//! whose span holds the sentinel is the current word (in full, not
//! just its prefix); a sentinel in a whitespace gap inserts an empty
//! word at the gap; a sentinel at the end after whitespace appends a
//! trailing empty word.
//!
//! Oracle: GNU bash 5.3.20 on a real pty (`complete -F` echoing
//! "${COMP_WORDS[*]}|${COMP_CWORD}|${COMP_WORDS[COMP_CWORD]}"):
//! - `mycmd al rest more` (cursor after `al`) -> N=4, CWORD=1, cur `al`
//!   (full token, words after the cursor present)
//! - `mycmd ` -> (mycmd ""), CWORD=1 (trailing empty word)
//! - `mycmd␣␣x` (cursor between the spaces) -> (mycmd "" x), CWORD=1
//!   (empty word inserted at the gap)
//! - `mycmd x` (cursor immediately before `x`) -> (mycmd x), CWORD=1,
//!   cur `x` (the token whose span holds the sentinel claims it)
//! - `X=1 mycmd sp` -> [mycmd sp] (assignments are not in the list)

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
        r#"_p() { COMPREPLY=("${COMP_WORDS[*]}|${COMP_CWORD}|${COMP_WORDS[COMP_CWORD]}"); }
           complete -F _p mycmd"#,
    );
    executor.complete_line(line, cursor).join("\n")
}

#[test]
fn issue_2726_words_after_cursor_are_present() {
    // Cursor right after `al` (byte 8); GNU reports all four words
    // and the FULL current token.
    assert_eq!(
        probe_words("mycmd al rest more", 8),
        "mycmd al rest more|1|al"
    );
}

#[test]
fn issue_2726_trailing_empty_word_is_present() {
    assert_eq!(probe_words("mycmd ", 6), "mycmd |1|");
}

#[test]
fn issue_2726_gap_cursor_completes_an_empty_word() {
    // Two spaces: cursor between them — split_at_delims inserts an
    // empty word at the gap (subst.c:2616: cw unset and
    // sentinel < ts of the next token). Probed on GNU bash 5.3.20:
    // CWORD=1 with an empty current word.
    assert_eq!(probe_words("mycmd  x", 6), "mycmd  x|1|");
    // One space, cursor immediately before `x`: the token whose span
    // holds the sentinel CLAIMS it (subst.c:2616: ts <= sentinel <=
    // te), so the list is [mycmd x] with the full `x` as the current
    // word — probed on GNU bash 5.3.20 (CWORD=1 N=2 WORDS=[mycmd x]
    // CUR=[x]). No empty word is inserted at a token's exact start.
    assert_eq!(probe_words("mycmd x", 6), "mycmd x|1|x");
}

#[test]
fn issue_2726_assignments_are_not_in_the_word_list() {
    assert_eq!(probe_words("X=1 mycmd sp", 12), "mycmd sp|1|sp");
}

#[test]
fn issue_2726_later_segments_words_do_not_leak_in() {
    // The full shape is per-SEGMENT: the word list ends at the
    // command's end (pcomplete.c:1323 substring end = find_cmd_end).
    assert_eq!(probe_words("mycmd al | other o", 8), "mycmd al|1|al");
}

#[test]
fn issue_2726_plain_prefix_shape_is_unchanged() {
    assert_eq!(probe_words("mycmd sp", 8), "mycmd sp|1|sp");
}
