//! Issue unixwin/rubash#1538: the `[[ ]]` argument slots reject
//! paren tokens the way GNU's cond_term does —
//!  1. `[[ a == )abc ]]` was silently accepted (the `)' fragment
//!     started the merged pattern word and the conditional evaluated
//!     false, rc 1); GNU delivers `)' as its own token and reports
//!     `unexpected argument `)' to conditional binary operator`, rc 2.
//!  2. `[[ a == ((a ]]` reported a generic near shape instead of
//!     `unexpected argument `(' to conditional binary operator` with
//!     near `((`.
//!
//! GNU owners:
//!  - parse.y:5196-5227 cond_term: after a binary operator the RHS is
//!    ONE read_token; any non-WORD token reports `unexpected argument
//!    `%s' to conditional binary operator` (etext from
//!    error_token_from_token). parse.y:5125-5143: the same for a unary
//!    operator's argument (`to conditional unary operator`).
//!  - parse.y:3663 + 5305 read_token/read_token_word under
//!    PST_REGEXP: after `=~' the reader jumps straight to word
//!    reading, where a leading `)' is a shell break character
//!    (parse.y:5688) — the RHS word is EMPTY, the `)' stays in the
//!    stream, and cond_error's generic form reports it
//!    (`syntax error in conditional expression: unexpected token`).
//!  - parse.y:6772-6808 error_token_from_text: the near token runs
//!    from the offending token through at most ONE extra character.
//!
//! Verified against GNU Bash 5.3.20 (-c probes): every expectation
//! below is byte-identical after stripping the program-name prefix.

use std::process::{Command, Stdio};

fn run_rubash(args: &[&str]) -> (String, String, Option<i32>) {
    let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

/// Diagnostic lines with the program-name prefix stripped: every line
/// reduces to its `line N: ...` body, the part GNU pins.
fn diag_lines(stderr: &str) -> Vec<String> {
    stderr
        .lines()
        .map(|line| match line.find("line ") {
            Some(at) => line[at..].to_string(),
            None => line.to_string(),
        })
        .collect()
}

fn expect_diags(stderr: &str, expected: &[&str]) {
    let expected: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
    assert_eq!(diag_lines(stderr), expected);
}

/// (input, diagnostic lines before the offending-line echo) — every
/// case also echoes its own source line and exits rc 2.
fn check_cases(cases: &[(&str, &[&str])]) {
    for (input, diags) in cases {
        let (stdout, stderr, code) = run_rubash(&["-c", input]);
        assert_eq!(stdout, "", "stdout for {input}");
        let mut expected: Vec<&str> = diags.to_vec();
        let echo = format!("line 1: `{input}'");
        expected.push(&echo);
        expect_diags(&stderr, &expected);
        assert_eq!(code, Some(2), "rc for {input}");
    }
}

#[test]
fn issue1538_binary_operator_slot_rejects_leading_parens() {
    check_cases(&[
        // The two forms the issue names.
        (
            "[[ a == )abc ]]",
            &[
                "line 1: unexpected argument `)' to conditional binary operator",
                "line 1: syntax error near `)a'",
            ],
        ),
        (
            "[[ a == ((a ]]",
            &[
                "line 1: unexpected argument `(' to conditional binary operator",
                "line 1: syntax error near `(('",
            ],
        ),
        // The rest of the binary-operator surface: every binary
        // operator's argument slot behaves identically, and the near
        // token is the paren plus at most one extra character.
        (
            "[[ a == ) ]]",
            &[
                "line 1: unexpected argument `)' to conditional binary operator",
                "line 1: syntax error near `)'",
            ],
        ),
        (
            "[[ a == (a ]]",
            &[
                "line 1: unexpected argument `(' to conditional binary operator",
                "line 1: syntax error near `(a'",
            ],
        ),
        (
            "[[ a == ( ]]",
            &[
                "line 1: unexpected argument `(' to conditional binary operator",
                "line 1: syntax error near `('",
            ],
        ),
        (
            "[[ a == (( ]]",
            &[
                "line 1: unexpected argument `(' to conditional binary operator",
                "line 1: syntax error near `(('",
            ],
        ),
        (
            "[[ a == (a|b) ]]",
            &[
                "line 1: unexpected argument `(' to conditional binary operator",
                "line 1: syntax error near `(a'",
            ],
        ),
        (
            "[[ a == (b) ]]",
            &[
                "line 1: unexpected argument `(' to conditional binary operator",
                "line 1: syntax error near `(b'",
            ],
        ),
        (
            "[[ a != (x ]]",
            &[
                "line 1: unexpected argument `(' to conditional binary operator",
                "line 1: syntax error near `(x'",
            ],
        ),
        (
            "[[ a < )x ]]",
            &[
                "line 1: unexpected argument `)' to conditional binary operator",
                "line 1: syntax error near `)x'",
            ],
        ),
        (
            "[[ a > (x ]]",
            &[
                "line 1: unexpected argument `(' to conditional binary operator",
                "line 1: syntax error near `(x'",
            ],
        ),
        (
            "[[ a -eq ( ]]",
            &[
                "line 1: unexpected argument `(' to conditional binary operator",
                "line 1: syntax error near `('",
            ],
        ),
        (
            "[[ a -nt ) ]]",
            &[
                "line 1: unexpected argument `)' to conditional binary operator",
                "line 1: syntax error near `)'",
            ],
        ),
        // The slot rule applies at any nesting level and after && / !.
        (
            "[[ x && a == )y ]]",
            &[
                "line 1: unexpected argument `)' to conditional binary operator",
                "line 1: syntax error near `)y'",
            ],
        ),
        (
            "[[ x && a == ((a ]]",
            &[
                "line 1: unexpected argument `(' to conditional binary operator",
                "line 1: syntax error near `(('",
            ],
        ),
        (
            "[[ ! a == )x ]]",
            &[
                "line 1: unexpected argument `)' to conditional binary operator",
                "line 1: syntax error near `)x'",
            ],
        ),
    ]);
}

#[test]
fn issue1538_grouped_argument_error_also_reports_expected_close() {
    // Inside a `( ... )' group the argument error is followed by the
    // group's own missing-close report (parse.y:5100-5109 shape).
    let (stdout, stderr, code) = run_rubash(&["-c", "[[ ( a == )x ) ]]"]);
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: unexpected argument `)' to conditional binary operator",
            "line 1: expected `)'",
            "line 1: syntax error near `)x'",
            "line 1: `[[ ( a == )x ) ]]'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1538_unary_operator_slot_rejects_parens() {
    check_cases(&[
        (
            "[[ -e ) ]]",
            &[
                "line 1: unexpected argument `)' to conditional unary operator",
                "line 1: syntax error near `)'",
            ],
        ),
        (
            "[[ -n ( ]]",
            &[
                "line 1: unexpected argument `(' to conditional unary operator",
                "line 1: syntax error near `('",
            ],
        ),
    ]);
}

#[test]
fn issue1538_regexp_rhs_leading_paren_uses_empty_word_form() {
    // After `=~' a leading bare `)' is NOT an argument error: the
    // regex word is empty and the `)' trips cond_error's generic form.
    check_cases(&[
        (
            "[[ a =~ ) ]]",
            &[
                "line 1: syntax error in conditional expression: unexpected token `)'",
                "line 1: syntax error near `)'",
            ],
        ),
        (
            "[[ a =~ )abc ]]",
            &[
                "line 1: syntax error in conditional expression: unexpected token `)'",
                "line 1: syntax error near `)a'",
            ],
        ),
        // A `)' after regex word characters ends the word the same way.
        (
            "[[ a =~ a) ]]",
            &[
                "line 1: syntax error in conditional expression: unexpected token `)'",
                "line 1: syntax error near `a)'",
            ],
        ),
    ]);
}

#[test]
fn issue1538_mid_word_paren_after_pattern_rhs_is_generic_error() {
    // Parens that arrive AFTER pattern word characters are separate
    // tokens tripping the generic form (the word ended before them).
    check_cases(&[
        (
            "[[ a == a) ]]",
            &[
                "line 1: syntax error in conditional expression: unexpected token `)'",
                "line 1: syntax error near `a)'",
            ],
        ),
        (
            "[[ a == a(b) ]]",
            &[
                "line 1: syntax error in conditional expression: unexpected token `('",
                "line 1: syntax error near `a(b'",
            ],
        ),
    ]);
}

#[test]
fn issue1538_valid_paren_forms_stay_silent() {
    // A quoted paren is a word; extglob groups after a pattern
    // character join the pattern; grouping parens group.
    let (stdout, stderr, code) = run_rubash(&["-c", "[[ a == \")\" ]]"]);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(1));
    let (stdout, stderr, code) = run_rubash(&["-c", "[[ ab == @(a)b ]]"]);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    let (stdout, stderr, code) = run_rubash(&["-c", "[[ ( a ) ]]"]);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
