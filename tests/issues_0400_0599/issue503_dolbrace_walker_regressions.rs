//! niubash#503 follow-up: the `${`-in-word LEX_WASDOL arm of the subscript
//! walkers (skip.rs `unclosed_array_subscript_walk`, continuation.rs
//! `subscript_residuals_advance`) was deleted as dead code — born
//! unreachable in 2183fc9c. `word` can only end with `$` when the `$` sits
//! at chars[index-1], but an adjacent `${` is always consumed earlier at
//! the `$`; every path that skips the match clears `word`. The arm also
//! violated the chars-API contract (`scan_braced_parameter_*` needs `${` at
//! slice[0..2]; it was handed a slice starting at `{`).
//!
//! These tests pin the LIVE contract around the removal: `${...}` bodies
//! never open subscripts (GNU parse.y:4126-4136 LEX_WASDOL / parse.y:5513
//! P_FIRSTCLOSE|P_DOLBRACE) while a real unclosed `[` later in the input is
//! still reported. Oracle: WSL GNU bash 5.3.20 script-file probes
//! (2026-10-23) — `x=${a[0]}; y[z` reports `]'` rc=2; `echo ${a[0]}` runs
//! clean.

use rubash::lexer::unclosed_array_subscript_line;

/// A closed `${...}` is consumed atomically; a later command-position `[`
/// still opens a subscript and must be reported — the arm previously bailed
/// at the `${` and never reached `y[z`.
#[test]
fn dolbrace_then_unclosed_subscript_is_reported() {
    assert_eq!(
        unclosed_array_subscript_line("x=${a[0]}; y[z"),
        Some((1, false)),
        "the `[` after a closed ${{...}} must still be reported"
    );
}

/// Same shape multi-line: report and construct lines track the later `[`.
#[test]
fn dolbrace_then_unclosed_subscript_reports_its_own_line() {
    assert_eq!(
        unclosed_array_subscript_line("x=${a[0]}\ny[z"),
        Some((2, false))
    );
}

/// Control: the `[` inside a complete `${...}` stays word material — no
/// subscript is opened and nothing is reported (GNU `echo ${a[0]}` runs).
#[test]
fn closed_dolbrace_subscript_text_is_not_reported() {
    assert_eq!(unclosed_array_subscript_line("echo ${a[0]}"), None);
    assert_eq!(unclosed_array_subscript_line("x=${a[0]}"), None);
}

/// Control: the identifier rule still gates — `x` after `=` is not a
/// command-position word, so `[` stays literal.
#[test]
fn assignment_value_bracket_stays_literal() {
    assert_eq!(unclosed_array_subscript_line("x=${a[0]}; y=x[z"), None);
}
