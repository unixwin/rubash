//! rubash#3307: the REPL completeness face still judged the gate-released
//! unclosed-case comsub family as unclosed.
//!
//! PR #3303 released the family at the non-interactive completeness gate
//! (`unclosed_input_close_char_posix`): a `)` arriving while the armed
//! pattern region has no content yet — or while a `|` separator has just
//! started a fresh pattern — closes the substitution for the body parser
//! instead of being held as case syntax. The interactive face
//! (`has_unclosed_input_syntax_posix`, consulted by the stdin feeder
//! `stdin_source_needs_more`) still held the separator-tail shapes to
//! PS2: the three `#2784`-witness unit scans (`Lexer::skip_cmd_subst`,
//! `skip_parenthesized_unit_corrected`, and its staged mirror
//! `skip_parenthesized_unit_corrected_ex`) carried no equivalent of the
//! gate's per-pattern content bookkeeping, so the `)` after `a|` read as
//! case syntax and the unit never balanced.
//!
//! The fix ports the gate/parser arm's bookkeeping (per-pattern content,
//! pattern-paren depth, opener availability, plus a pending-opener count
//! standing in for the gate's pushed `(` delimiter) into all three scans,
//! and adds the gate's reclassification predicate
//! `(region_before && !content_before)` to their `)` arms.
//!
//! Expectations verified against GNU bash 5.2.21 (interactive `-i` with a
//! piped line: every released shape errors immediately, no PS2 prompt;
//! `-c`: exit 127 with a syntax diagnostic). The opener shapes are the
//! pinned detention boundary, not a bug: the gate test
//! `unclosed_case_body_paren_closes_command_substitution` holds
//! `$(case x in ()` / `$(case x in (` / `$(case x in a;;)` — GNU also
//! errors immediately on them, but the gate (and hence this face)
//! deliberately keeps them held; this test pins the gate's verdict,
//! not GNU's, for those controls.
//!
//! NOT pinned here: the nested shape `echo $(echo $(case x in a|))`
//! (the gate itself holds it — its case machine is global across the
//! nested substitution, so the outer `)` reads as case syntax) and the
//! two-substitution combination `echo $(case x in a|) $(echo ok)` whose
//! `-c` abort is owned by the parser extractor's region (deferred in
//! the #3303 notes: the extractor's region never arms for the
//! separator-tail shapes, so the body error is reported but the driver
//! continues with status 0). Both are registered residuals, not this
//! issue's face.

use std::process::Command;

/// The released family: the gate releases these, so the REPL must judge
/// them complete (and then fail with the body grammar verdict), not hold
/// them open to PS2.
const RELEASED: &[&str] = &[
    "echo $(case x in)",
    "echo $(case x in )",
    "echo \"$(case x in)\"",
    "echo $(case x)",
    "echo $(case x in a|)",
    "echo $(case x in a| )",
    "case a in a) echo $(case x in);; esac",
    // Further shapes the gate releases through the same predicate.
    "echo $(case x in |)",
    "echo $(case x in a||)",
    "echo $(case x in a|;&)",
    "echo $(case x in ;&)",
];

/// Controls: the gate holds these, so the interactive face must hold
/// them too (PS2 continuation, not an immediate verdict).
const HELD: &[&str] = &[
    // Real pattern content present.
    "echo $(case x in a) echo x)",
    "echo $(case x in a|b)",
    "echo $(case x in a|b|c)",
    "echo $(case x in a|esac)",
    "echo $(case x in a) echo hi | cat)",
    // Pattern-list opener: the `)` is owned by the opener.
    "echo $(case x in ()",
    "echo $(case x in (",
    "echo $(case x in (a|)",
    "echo $(case x in (|)",
    "echo $(case x in a(b|)",
    "echo $(case x in (a)",
    "echo $(case x in (a esac)",
    // Clause terminator with no fresh pattern begun.
    "echo $(case x in a;;)",
    // `&` is pattern content, not a separator.
    "echo $(case x in &)",
];

#[test]
fn released_family_is_complete_at_the_interactive_face() {
    for src in RELEASED {
        assert!(
            !rubash::lexer::has_unclosed_input_syntax_posix(src, false),
            "interactive face should judge the gate-released unit complete: {src}"
        );
        assert!(
            !rubash::script_driver::stdin_source_needs_more(src),
            "stdin feeder should not ask for a continuation line: {src}"
        );
    }
}

#[test]
fn held_controls_stay_unclosed_at_the_interactive_face() {
    for src in HELD {
        assert!(
            rubash::lexer::has_unclosed_input_syntax_posix(src, false),
            "interactive face should still hold the gate-held unit: {src}"
        );
        assert!(
            rubash::script_driver::stdin_source_needs_more(src),
            "stdin feeder should still ask for a continuation line: {src}"
        );
    }
}

fn dash_c(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash -c");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn released_family_fails_with_the_body_grammar_verdict() {
    // The released units are complete input, so `-c` hands them to the
    // body parser, which rejects the unclosed case at the `)` (GNU: 127
    // with `syntax error near unexpected token`).
    for src in [
        "echo $(case x in)",
        "echo $(case x in )",
        "echo $(case x)",
        "echo $(case x in a|)",
        "echo $(case x in a| )",
        "echo $(case x in |)",
        "echo $(case x in a||)",
        "echo $(case x in a|;&)",
        "echo $(case x in ;&)",
        "case a in a) echo $(case x in);; esac",
    ] {
        let (_stdout, stderr, code) = dash_c(src);
        assert_eq!(code, Some(127), "-c should abort 127: {src}\n{stderr}");
        assert!(
            stderr.contains("syntax error"),
            "-c should report a syntax error: {src}\n{stderr}"
        );
    }
}

#[test]
fn complete_cases_still_run() {
    // `echo` with an empty substitution still prints its newline.
    for (src, expected_stdout) in [
        ("echo $(case x in a|b) x;; esac)", "\n"),
        ("echo $(case x in esac)", "\n"),
        ("echo $(echo hi)", "hi\n"),
    ] {
        let (stdout, stderr, code) = dash_c(src);
        assert_eq!(code, Some(0), "{src}\n{stderr}");
        assert_eq!(stdout, expected_stdout, "{src}");
    }
}
