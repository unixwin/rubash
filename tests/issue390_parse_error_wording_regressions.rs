//! rubash#390: parse-error wording/rc divergences vs GNU 5.3.0 — four
//! focused families, all byte-verified against WSL GNU Bash 5.3.0
//! script-file probes (artifacts:
//! target/issue-suites/results/wt39-caseerr/issue390-probes/, 2026-10-02).
//!
//! GNU spec anchors:
//! * q1 — `((X=([))]`-class: read_token hands `((` to parse_dparen
//!   (parse.y:3726-3733) → parse_arith_cmd's P_ARITH matched pair
//!   (parse.y:4970). EOF with the paren count open reports `unexpected
//!   EOF while looking for matching `)'' at the group's line (parse.y:3908
//!   -3915) with exit 2 (`error yacc_EOF' forces EX_BADUSAGE only on a
//!   still-zero status, parse.y:484-490). A group that CLOSES with the
//!   next char != `)' is reinterpreted as a nested subshell (parse.y:4938
//!   -4948); when its reparse leaves `name=(` open at clean EOF,
//!   parse_compound_assignment reports the same `)' wording at the
//!   compound's line with set_exit_status(EXECUTION_FAILURE) (parse.y:7140
//!   -7174) — exit 1, kept by parse.y:489-490.
//! * q2 — an unterminated quote INSIDE a `[` array subscript is the quote
//!   recursion's own EOF (parse.y:4040-4051): the closer char and report
//!   line are the QUOTE's (`a["x]=15` → `"' at the `"` line).
//! * q3 — the empty-case `esac' (parse.y:3433-3441) closes the case; the
//!   enclosing subshell absorbs the following `)' and the error surfaces
//!   at the NEXT command-start token (`syntax error near unexpected token
//!   `printf'', the yacc grammar's rejection via error_token_from_token,
//!   parse.y:6850-6865).
//! * q4 — after the finished case, `|' is a pipeline connector and the
//!   reserved word that follows is the FI/ESAC token the grammar rejects:
//!   `case esac in esac|fi) ...` names `fi'.
//! * The reserved-word abort (q4's engine) is parse.y:469-482's `error
//!   '\n'' → YYABORT: the reader stops, so no later token of the input
//!   overrides the report and no later script line runs.

use std::process::Command;

fn rubash(script: &str) -> (String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stderr).replace("bash: -c: line ", "line "),
        output.status.code(),
    )
}

#[test]
fn q1_arith_eof_names_close_paren_at_open_line_rc2() {
    // P_ARITH group never closes (`((1+(2`, `((x=(`, `((1+2`).
    for script in ["((1+(2", "((x=(", "((1+2", "((x=()"] {
        let (stderr, code) = rubash(script);
        assert_eq!(
            stderr, "line 1: unexpected EOF while looking for matching `)'\n",
            "{script}"
        );
        assert_eq!(code, Some(2), "{script}");
    }
}

#[test]
fn q1_dparen_subshell_reparse_compound_eof_rc1() {
    // The group closes but the next char is not `)`: the nested-subshell
    // reparse leaves `X=(` open at clean EOF (parse.y:7140-7174).
    for script in ["((X=([))]", "((X=([))]x"] {
        let (stderr, code) = rubash(script);
        assert_eq!(
            stderr, "line 1: unexpected EOF while looking for matching `)'\n",
            "{script}"
        );
        assert_eq!(code, Some(1), "{script}");
    }
}

#[test]
fn q1_dparen_contrast_shapes_keep_their_wording() {
    // Unterminated `[` inside the reparse: the `]' family (probe
    // `((x=([y))`); fully-closed inner subshell: the eof-from-`(' family
    // (probe `((x=(y))`).
    let (stderr, code) = rubash("((x=([y))");
    assert_eq!(
        stderr,
        "line 1: unexpected EOF while looking for matching `]'\n"
    );
    assert_eq!(code, Some(1));

    let (stderr, code) = rubash("((x=(y))");
    assert_eq!(
        stderr,
        "line 2: syntax error: unexpected end of file from `(' command on line 1\n"
    );
    assert_eq!(code, Some(2));
}

#[test]
fn q1_arith_brackets_are_evaluator_data() {
    // GNU's P_ARITH scan counts parens only — brackets flow to the
    // evaluator (rc 1), never to a parse-status rejection.
    let (stderr, code) = rubash("((x=[y))");
    assert_eq!(
        stderr,
        "bash: line 1: ((: x=[y: arithmetic syntax error: operand expected (error token is \"[y\")\n"
    );
    assert_eq!(code, Some(1));

    let (stderr, code) = rubash("((a[b))");
    assert_eq!(
        stderr,
        "bash: line 1: ((: a[b: bad array subscript (error token is \"a[b\")\n"
    );
    assert_eq!(code, Some(1));
}

#[test]
fn q2_subscript_quote_eof_names_the_quote() {
    // parse.y:4040-4051: the quote recursion's EOF owns the report —
    // closer char and open line are the QUOTE's.
    let (stderr, code) = rubash("a[\"x]=15\necho after");
    assert_eq!(
        stderr,
        "line 1: unexpected EOF while looking for matching `\"'\n"
    );
    assert_eq!(code, Some(2));

    let (stderr, code) = rubash("a[\n\"x");
    assert_eq!(
        stderr,
        "line 2: unexpected EOF while looking for matching `\"'\n"
    );
    assert_eq!(code, Some(2));

    // rc 1 inside a compound assignment (parse_compound_assignment family).
    let (stderr, code) = rubash("x=([\"y\nz");
    assert_eq!(
        stderr,
        "line 1: unexpected EOF while looking for matching `\"'\n"
    );
    assert_eq!(code, Some(1));

    // The `]' wording stays for a bracket-shaped EOF.
    let (stderr, code) = rubash("a[\nb");
    assert_eq!(
        stderr,
        "line 1: unexpected EOF while looking for matching `]'\n"
    );
    assert_eq!(code, Some(2));
}

#[test]
fn q3_subshell_absorbs_close_paren_and_names_next_word() {
    // B12: `( case esac in\nesac) printf matched ;; esac )' — the empty
    // case ends at the bare `esac' (parse.y:3433-3441), the `)' closes the
    // SUBSHELL, and the completed subshell followed by `printf' without a
    // separator is the yacc error (probe: names `printf', line 2).
    let (stderr, code) = rubash("( case esac in\nesac) printf matched ;; esac )");
    assert_eq!(
        stderr,
        "line 2: syntax error near unexpected token `printf'\nline 2: `esac) printf matched ;; esac )'\n"
    );
    assert_eq!(code, Some(2));

    // At top level the same `)' is the stray token (E01 family).
    let (stderr, code) = rubash("case esac in esac) echo single ;; esac");
    assert_eq!(
        stderr,
        "line 1: syntax error near unexpected token `)'\nline 1: `case esac in esac) echo single ;; esac'\n"
    );
    assert_eq!(code, Some(2));
}

#[test]
fn q4_pipeline_tail_reserved_word_is_named() {
    // E02: after the empty case, `|' continues the pipeline and the grammar
    // rejects the FI/ESAC token (error_token_from_token, parse.y:6850).
    let (stderr, code) = rubash("case esac in esac|fi) echo hi ;; esac");
    assert_eq!(
        stderr,
        "line 1: syntax error near unexpected token `fi'\nline 1: `case esac in esac|fi) echo hi ;; esac'\n"
    );
    assert_eq!(code, Some(2));

    let (stderr, code) = rubash("case x in esac|esac) echo hi ;; esac");
    assert!(
        stderr.contains("syntax error near unexpected token `esac'"),
        "{stderr}"
    );
    assert_eq!(code, Some(2));

    // The general ordering rule (no case needed): the reserved word after
    // `|' is rejected before any later `)' is even read.
    let (stderr, code) = rubash("echo a | fi )");
    assert_eq!(
        stderr,
        "line 1: syntax error near unexpected token `fi'\nline 1: `echo a | fi )'\n"
    );
    assert_eq!(code, Some(2));
}

#[test]
fn reserved_word_error_stops_the_reader() {
    // parse.y:469-482 (`error '\n'' → YYABORT): after the fi rejection the
    // remaining script lines never run (probe m5/m6: rc 2, no `ok').
    let (stdout, stderr, code) = {
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg("-c")
            .arg("echo a | fi\necho ok")
            .output()
            .expect("run rubash");
        (
            String::from_utf8_lossy(&output.stdout).to_string(),
            String::from_utf8_lossy(&output.stderr).replace("bash: -c: line ", "line "),
            output.status.code(),
        )
    };
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "line 1: syntax error near unexpected token `fi'\nline 1: `echo a | fi'\n"
    );
    assert_eq!(code, Some(2));
}

#[test]
fn legal_neighbors_stay_legal() {
    // Both directions of the class boundary: valid shapes must not regress.
    for script in [
        "case x in fi) echo hi ;; esac",
        "case x in (esac) echo hi ;; esac",
        "case x in a|esac) echo hi ;; esac",
        "case x in esac-text) echo prefixed ;; esac; echo done",
        "case x in esac | cat",
        "for ((i=0;i<2;i++)); do echo $i; done",
        "( echo hi ) ; printf matched",
        "a[x]=15; echo ${a[x]}",
    ] {
        let (stderr, code) = rubash(script);
        assert_eq!(stderr, "", "{script}");
        assert_eq!(code, Some(0), "{script}");
    }
}
