//! rubash#413: case/compound diagnostic residuals batch 2 (from the
//! #390/#391 matrix expansion), all byte-verified against WSL GNU Bash 5.3.0
//! script-file probes (artifacts:
//! target/issue-suites/results/wt48-resid413/, 2026-10-03).
//!
//! GNU spec anchors:
//! * read_token_word's subscript arm (parse.y:5635-5651): an element-LEADING
//!   `[` under PST_COMPASSIGN consumes the matched `[...]` span via
//!   parse_matched_pair('[', ']', P_ARRAYSUB) — the span's interior, spaces
//!   and PARENS included, is word data. A `)` inside `[)]` can never close
//!   the compound list (parse.y:7140's loop), so `(X=([)])` leaves the
//!   subshell open at EOF and reports "unexpected end of file from `('
//!   command" (parse.y:6892-6901) with exit 2.
//! * parse_dparen (parse.y:3726-3733 → 4895-4948): the arith-vs-subshell
//!   verdict is a CHARACTER-level parse_matched_pair('(', ')', P_ARITH)
//!   scan (parse.y:4970) that counts every unquoted paren — including
//!   parens inside a compound-assignment body the word layer folds. The
//!   construct is arithmetic only when the char right after the count-zero
//!   `)` is `)` (parse.y:4976); otherwise `((` re-lexes as nested
//!   subshells: `(( x=([))] ))` dies with the compound's clean-EOF report
//!   `unexpected EOF while looking for matching `)'' exit 1 (parse.y:7140
//!   -7152), and `((X=([a]))]` dies on the grammar — a word directly after
//!   the closed nested subshell has no separator (parse.y:1097 subshell →
//!   1252-1279 compound_list) — "syntax error near unexpected token `]'"
//!   plus the offending-line echo (parse.y:6850-6865), exit 2.
//! * The declare operand path (expand_compound_array_assignment,
//!   arrayfunc.c:557 → 610 expand_words_no_vars → subst.c:13205): field
//!   splitting applies only to UNQUOTED EXPANSION RESULTS, never to the
//!   literal word bytes — `declare -a d=([ empty ] =)` stores the two
//!   elements `[ empty ]` and `=` exactly like the bare `arr=` path.

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

fn rubash_stdout(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).replace("bash: -c: line ", "line "),
        output.status.code(),
    )
}

/// Issue form 1: the `((` group closes at the `)` before `]`, so the char
/// after it is not `)` — nested-subshell reinterpretation. The reparse's
/// `x=([))] )` word keeps the compound open (its element bracket swallowed
/// one `)`), the pushed string's own `)` closes only the inner subshell, and
/// EOF names the outer `(` (parse.y:6892-6901) with exit 2.
#[test]
fn form1_dparen_compound_subscript_eof_from_paren() {
    let (stderr, code) = rubash("(( x=([))] ))");
    assert_eq!(
        stderr,
        "line 2: syntax error: unexpected end of file from `(' command on line 1\n"
    );
    assert_eq!(code, Some(2));

    // With a following line the eof report moves to the EOF line and
    // nothing runs.
    let (stdout, stderr, code) = rubash_stdout("(( x=([))] ))\necho after");
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "line 3: syntax error: unexpected end of file from `(' command on line 1\n"
    );
    assert_eq!(code, Some(2));

    // One extra `)` balances the reinterpretation: accepted, rc 0.
    let (stdout, stderr, code) = rubash_stdout("(( x=([))] )) )\necho after");
    assert_eq!(stdout, "after\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));

    // Two extra: the stray `)` is the near token instead.
    let (stderr, code) = rubash("(( x=([))] ))))");
    assert_eq!(
        stderr,
        "line 1: syntax error near unexpected token `)'\nline 1: `(( x=([))] ))))'\n"
    );
    assert_eq!(code, Some(2));
}

/// Issue form 2: the group closes and the char after is `]` — the
/// reinterpretation closes the inner subshell, and the trailing `]` word
/// follows a complete command with no separator: the grammar names it.
#[test]
fn form2_dparen_stray_word_after_nested_subshell() {
    for script in ["((X=([a]))]", "(( X=([a])) ]", "((X=([a]))] ; echo ok"] {
        let (stderr, code) = rubash(script);
        assert_eq!(
            stderr,
            format!("line 1: syntax error near unexpected token `]'\nline 1: `{script}'\n"),
            "{script}"
        );
        assert_eq!(code, Some(2), "{script}");
    }
}

/// Issue form 3: the element-leading `[` scan eats the `)` that would close
/// the compound, so the subshell never closes — eof-from-`(' at the EOF
/// line, rc 2, and no following line runs.
#[test]
fn form3_compound_subscript_swallows_close_paren() {
    let (stdout, stderr, code) = rubash_stdout("(X=([)])\necho after");
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "line 3: syntax error: unexpected end of file from `(' command on line 1\n"
    );
    assert_eq!(code, Some(2));

    // The balanced text-level shapes around it: one extra `)` accepts,
    // a missing one reports the compound's own EOF, and the bare word
    // after the closed nested subshell is the near token.
    let (stdout, _, code) = rubash_stdout("( x=([)]) )\necho after");
    assert_eq!(stdout, "after\n");
    assert_eq!(code, Some(0));

    let (stderr, code) = rubash("x=([)]");
    assert_eq!(
        stderr,
        "line 1: unexpected EOF while looking for matching `)'\n"
    );
    assert_eq!(code, Some(1));

    let (stdout, stderr, code) = rubash_stdout("( (echo hi) x\necho after");
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "line 1: syntax error near unexpected token `x'\nline 1: `( (echo hi) x'\n"
    );
    assert_eq!(code, Some(2));

    let (stderr, code) = rubash("(x=([)]");
    assert_eq!(
        stderr,
        "line 1: unexpected EOF while looking for matching `)'\n"
    );
    assert_eq!(code, Some(1));
}

/// Issue form 4: the declare operand pipeline stores the same element
/// words as the bare-assignment path — a trigger-free element never field
/// splits (subst.c:13205 splits expansion results only), so `[ empty ]`
/// stays one element and `=` the next.
#[test]
fn form4_declare_compound_bracket_span_stays_one_element() {
    let (stdout, stderr, code) = rubash_stdout(
        "declare -a d=([ empty ] =)\necho \"rc=$? n=${#d[@]} [0]=${d[0]-U} [1]=${d[1]-U}\"",
    );
    assert_eq!(stdout, "rc=0 n=2 [0]=[ empty ] [1]==\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));

    let (stdout, _, code) = rubash_stdout("declare -a d=([ empty ] =)\ndeclare -p d");
    assert_eq!(stdout, "declare -a d=([0]=\"[ empty ]\" [1]=\"=\")\n");
    assert_eq!(code, Some(0));

    // Quote removal and pathname expansion still own the element: a quoted
    // interior keeps its data semantics, and an unresolvable pattern stays
    // literal (arrayfunc.c:610's tail — the unmatched-suffix pattern is
    // cwd-independent).
    let (stdout, _, code) =
        rubash_stdout("declare -a d=([ \"a]=b\" ] =)\necho \"rc=$? n=${#d[@]} [0]=${d[0]-U}\"");
    assert_eq!(stdout, "rc=0 n=2 [0]=[ a]=b ]\n");
    assert_eq!(code, Some(0));

    let (stdout, _, code) =
        rubash_stdout("declare -a d=([a b]zz*)\necho \"rc=$? n=${#d[@]} [0]=${d[0]-U}\"");
    assert_eq!(stdout, "rc=0 n=1 [0]=[a b]zz*\n");
    assert_eq!(code, Some(0));

    // Expansion-bearing elements still field-split their results.
    let (stdout, _, code) =
        rubash_stdout("v=\"p q\"\ndeclare -a d=($v [ empty ] =)\necho \"n=${#d[@]}\"");
    assert_eq!(stdout, "n=4\n");
    assert_eq!(code, Some(0));
}

/// The dparen verdict must stay char-level: arithmetic commands with
/// compound-looking bodies still parse as arithmetic when the group really
/// ends in `))` (the evaluator, not the parser, owns their diagnostics).
#[test]
fn dparen_verdict_arithmetic_side_stands() {
    let (stdout, stderr, code) = rubash_stdout("x=5\n(( x=([0]=3) )) ; echo \"rc=$? x=$x\"");
    assert_eq!(stdout, "rc=1 x=5\n");
    assert_eq!(
        stderr,
        "bash: line 2: ((: x=([0]=3) : arithmetic syntax error: operand expected (error token is \"[0]=3) \")\n"
    );
    assert_eq!(code, Some(0));

    let (stdout, _, code) = rubash_stdout("(( x=(1+2) ))\necho \"rc=$? x=${x:-U}\"");
    assert_eq!(stdout, "rc=0 x=3\n");
    assert_eq!(code, Some(0));

    // Fully-closed inner subshell keeps the eof-from-`(' family (rubash#390
    // contrast shape) and the never-closing group keeps its rc 2 report.
    let (stderr, code) = rubash("((x=(y))");
    assert_eq!(
        stderr,
        "line 2: syntax error: unexpected end of file from `(' command on line 1\n"
    );
    assert_eq!(code, Some(2));

    let (stderr, code) = rubash("((x=(");
    assert_eq!(
        stderr,
        "line 1: unexpected EOF while looking for matching `)'\n"
    );
    assert_eq!(code, Some(2));

    let (stderr, code) = rubash("((x=([y))");
    assert_eq!(
        stderr,
        "line 1: unexpected EOF while looking for matching `]'\n"
    );
    assert_eq!(code, Some(1));
}

/// Ordinary compound assignments keep their element integrity: subscripts,
/// nested brackets and the `name[sub]=(list)` spelling all still parse.
#[test]
fn ordinary_compound_assignments_keep_element_integrity() {
    let (stdout, stderr, code) = rubash_stdout(
        "a=([0]=x [1]=y)\necho \"${a[0]}${a[1]}\"\nb=([[a] b] =)\necho \"n=${#b[@]} [0]=${b[0]}\"",
    );
    assert_eq!(stdout, "xy\nn=2 [0]=[[a] b]\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));

    let (stdout, stderr, code) =
        rubash_stdout("arr=([ empty ] =)\necho \"n=${#arr[@]} [0]=${arr[0]}\"");
    assert_eq!(stdout, "n=2 [0]=[ empty ]\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
