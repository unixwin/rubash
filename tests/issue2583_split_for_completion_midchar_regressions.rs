//! rubash#2583 — `split_for_completion` panicked on a host-supplied
//! cursor that falls inside a multi-byte character: the host's
//! rl_point-equivalent byte offset was used to slice the line
//! (`&line[..cursor]`) after only a length clamp, so a
//! boundary-transient offset crashed the whole host shell.
//!
//! GNU ownership: readline only ever slices the line buffer at
//! character boundaries; the completion entry points treat rl_point
//! as a byte offset into a char stream, never as a char index. The
//! port clamps the offset down to the nearest char boundary
//! (`str::floor_char_boundary`), so a mid-char cursor behaves exactly
//! like the boundary cursor just before it.

use rubash::executor::Executor;
use rubash::lexer::tokenize;
use rubash::parser::parse;

fn run_script(executor: &mut Executor, script: &str) {
    let tokens = tokenize(script);
    let ast = parse(&tokens);
    let result = executor.execute_ast(&ast);
    assert!(result.is_ok(), "script failed: {script:?} -> {result:?}");
}

/// 'é' is two bytes (0xC3 0xA9): byte offset +1 inside it is the
/// mid-char cursor a host can legitimately hand over.
#[test]
fn mid_char_cursor_in_current_word_does_not_panic() {
    let mut executor = Executor::new();
    let line = "echo café";
    let e_start = line.find('é').expect("fixture has é");
    let mid = e_start + 1;
    assert!(!line.is_char_boundary(mid), "fixture cursor is mid-char");
    let from_mid = executor.complete_line(line, mid);
    let from_floor = executor.complete_line(line, e_start);
    assert_eq!(
        from_mid, from_floor,
        "a mid-char cursor must behave like the boundary just before it"
    );
}

#[test]
fn mid_char_cursor_in_earlier_word_does_not_panic() {
    let mut executor = Executor::new();
    run_script(
        &mut executor,
        "_f() { COMPREPLY=(hit); }; complete -F _f git",
    );
    let line = "git café ch";
    let e_start = line.find('é').expect("fixture has é");
    // Cursor at the very end (a boundary) but the completed line
    // contains a multi-byte char; and a cursor dropped inside the é
    // itself must both survive.
    let full = executor.complete_line(line, line.len());
    assert_eq!(full, vec!["hit".to_string()]);
    let from_mid = executor.complete_line(line, e_start + 1);
    let from_floor = executor.complete_line(line, e_start);
    assert_eq!(from_mid, from_floor);
}

#[test]
fn out_of_range_cursor_still_clamps() {
    let mut executor = Executor::new();
    let line = "echo café";
    let clamped = executor.complete_line(line, usize::MAX);
    let at_end = executor.complete_line(line, line.len());
    assert_eq!(clamped, at_end);
}
