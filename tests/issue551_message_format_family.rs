//! rubash#551 (C-family tracking): message-format cells, each byte-verified
//! against WSL GNU Bash 5.3.0 stdin probes (2026-10-12, probe artifacts
//! C:/Users/cmx/AppData/Local/Temp/msgfam/). The stdin driver binds argv[0]
//! as the diagnostic program name, so the harness normalizes the test
//! executable's path to `bash` exactly like the audit baseline's sed.
//!
//! GNU C-source anchors per cell:
//! * C3a — `echo ${a[`: parse.y:5513 read_token_word opens the braced
//!   parameter's parse_matched_pair (P_FIRSTCLOSE|P_DOLBRACE); the `[` after
//!   `a` is ordinary word text (the parse.y:4126 P_DOLBRACE dollar-recursion
//!   needs LEX_WASDOL — a `[` directly after `$`), so EOF names the braced
//!   `}' (probe: `}`), never `]'.
//! * C3b — `$((1+${${x`: the `$((` P_ARITH matched pair (parse.y:4467
//!   parse_comsub → parse_matched_pair P_ARITH) treats a non-funsub `${` as
//!   ordinary arithmetic text (parse.y:4126-4136 peek FUNSUB_CHAR), so EOF
//!   names the arithmetic `)' (probe: `)`).
//! * C4a/c — `[[ a =~ (b ]]` / `[[ x =~ (unclosed`: under PST_REGEXP
//!   (parse.y:5169) read_token_word folds the unquoted `(` group via
//!   parse_matched_pair (parse.y:5443); its EOF reports `)' at the `(' line
//!   (start_lineno, parse.y:3912) and sets PST_NOERROR (parse.y:3914), so
//!   the rhs rejection "unexpected argument to conditional binary operator"
//!   (parse.y:5227, EOF line) is the LAST diagnostic and yyerror's
//!   report_syntax_error tail is suppressed (parse.y:6724-6726) — two lines,
//!   no echo.
//! * C4b — `[ ( 1 -eq 1`: read_token's `(` after a WORD token is the
//!   function-definition `(' (parse.y:3774-3790 keeps PST_SUBSHELL off);
//!   the grammar shifts it expecting `)' and rejects the NEXT token:
//!   `syntax error near unexpected token `1'' plus print_offending_line
//!   (parse.y:6814), both at line 1.
//! * C5 — `echo one\necho ${x\n"<x`: the `"` opens inside the braced
//!   parameter's matched-pair scan; the quote recursion's EOF reports the
//!   QUOTE at its own line (start_lineno) and PST_NOERROR (parse.y:3914)
//!   suppresses every further report — ONE diagnostic, at line 3.
//! * C6 — `v=(<(echo`: the word-attached `<(` opens parse_comsub
//!   (parse.y:5494-5524 read_token_word shellexp arm); EOF inside takes the
//!   yyerror route with shell_eof_token `)' → "matching `)'" at the EOF
//!   line (probe: line 2, rc 1 via the array-list EOF family).
//! * C8 — `exec 3< f; echo x >&3`: redir.c:1153 dup2 succeeds for any open
//!   descriptor; the write fails at sh_chkwrite (builtins/common.c:320-334)
//!   → `<builtin>: write error: Bad file descriptor`, rc 1. A dup of a
//!   CLOSED fd still fails at setup (`3: Bad file descriptor`).
//! * C1 offset — the read-stage EOF scans number lines within the gathered
//!   group; the stdin diagnostic must equal the file-driven global line
//!   (GNU g2 probe: `echo A\necho B\necho "unclosed` reports line 3).

use std::io::Write;
use std::process::{Command, Stdio};

fn rubash_stdin(script: &str) -> (String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(script.as_bytes())
        .expect("write script");
    let output = child.wait_with_output().expect("run rubash");
    let exe = env!("CARGO_BIN_EXE_rubash").to_string();
    (
        String::from_utf8_lossy(&output.stderr)
            .replace(&exe, "bash")
            .replace(&exe.replace('/', "\\"), "bash"),
        output.status.code(),
    )
}

#[test]
fn c1_c2_stdin_syntax_error_carries_line_and_verbatim_echo() {
    // huck#211/#283: the representative C1 cell — `line 4:' present and the
    // offending-line echo byte-identical to the source line.
    let (stderr, code) = rubash_stdin("if true\nthen\necho x\nfi extra\n");
    assert_eq!(
        stderr,
        "bash: line 4: syntax error near unexpected token `extra'\n\
         bash: line 4: `fi extra'\n"
    );
    assert_eq!(code, Some(2));

    // huck#575/#284 (C2): `;;' rejection echoes the raw line, no re-spacing.
    let (stderr, code) = rubash_stdin("echo L2;;\n");
    assert_eq!(
        stderr,
        "bash: line 1: syntax error near unexpected token `;;'\n\
         bash: line 1: `echo L2;;'\n"
    );
    assert_eq!(code, Some(2));
}

#[test]
fn c1_read_stage_eof_reports_the_global_line_in_later_groups() {
    // The quote group starts at file line 3; the diagnostic must say 3, not
    // the group-local 1 (GNU probe g2: line 3).
    let (stderr, code) = rubash_stdin("echo A\necho B\necho \"unclosed\n");
    assert_eq!(
        stderr,
        "bash: line 3: unexpected EOF while looking for matching `\"'\n"
    );
    assert_eq!(code, Some(2));
}

#[test]
fn c3_eof_naming_subscript_inside_brace_and_arith() {
    // C3a (huck#635 subscript_cell): the `[` inside `${a` is ordinary
    // braced-word text — the `}' owns the EOF.
    let (stderr, code) = rubash_stdin("echo ${a[\n");
    assert_eq!(
        stderr,
        "bash: line 1: unexpected EOF while looking for matching `}'\n"
    );
    assert_eq!(code, Some(2));

    // C3b (huck#635 nested_in_arith): `${` never pushes inside the `$((`
    // arithmetic matched pair — the arith `)' owns the EOF.
    let (stderr, code) = rubash_stdin("echo $((1+${${x\n");
    assert_eq!(
        stderr,
        "bash: line 1: unexpected EOF while looking for matching `)'\n"
    );
    assert_eq!(code, Some(2));

    // The `${`-with-subscript and `${$`-shaped neighbors keep GNU's names.
    for (script, closer) in [
        ("echo ${a[0\n", "}"),
        ("echo ${x[${y\n", "}"),
        ("echo ${a[}${x\n", "}"),
        ("echo $((1+${a[0\n", ")"),
        ("echo $((1+${x\n", ")"),
    ] {
        let (stderr, _) = rubash_stdin(script);
        assert_eq!(
            stderr,
            format!("bash: line 1: unexpected EOF while looking for matching `{closer}'\n"),
            "{script}"
        );
    }
}

#[test]
fn c3b_funsub_and_dparen_neighbors_keep_their_routes() {
    // A `${ ` FUNSUB still opens its delimiter everywhere (the C3b skip is
    // the non-funsub-inside-arithmetic case ONLY): an unterminated funsub
    // keeps reading and reports the braced `}' at the EOF line, and the
    // -c dparen rc family is untouched (issue390 q1: `((1+2` exits 2 via
    // the P_ARITH EOF report, not the array-list exit 1).
    for script in [
        ">_[${ $() }] x\n",
        "echo ${ x }; echo ok\n",
        "echo >_[${ x }]\n",
    ] {
        let (stderr, _) = rubash_stdin(script);
        assert_eq!(
            stderr, "bash: line 2: unexpected EOF while looking for matching `}'\n",
            "{script}"
        );
    }
    for script in ["((1+2", "((x=(", "((1+(2", "((x=()"] {
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg("-c")
            .arg(script)
            .output()
            .expect("run rubash");
        assert_eq!(output.status.code(), Some(2), "{script}");
    }
}

#[test]
fn c4a_c_regex_unclosed_paren_two_part_diagnostic() {
    for script in ["[[ a =~ (b ]]\n", "[[ x =~ (unclosed\n"] {
        let (stderr, code) = rubash_stdin(script);
        assert_eq!(
            stderr,
            "bash: line 1: unexpected EOF while looking for matching `)'\n\
             bash: line 2: unexpected argument to conditional binary operator\n",
            "{script}"
        );
        assert_eq!(code, Some(2), "{script}");
    }
}

#[test]
fn c4b_bracket_open_paren_is_a_function_def_paren() {
    // The `(' after the WORD `[' expects `)'; `1' is rejected at line 1 with
    // the verbatim line echo — never an EOF-from-`(' wait.
    let (stderr, code) = rubash_stdin("[ ( 1 -eq 1\n");
    assert_eq!(
        stderr,
        "bash: line 1: syntax error near unexpected token `1'\n\
         bash: line 1: `[ ( 1 -eq 1'\n"
    );
    assert_eq!(code, Some(2));
}

#[test]
fn c5_operand_spanning_quote_reports_one_line() {
    // GNU prints ONE diagnostic: the quote's own EOF at line 3. The braced
    // construct is never reported separately and the prefix cut runs only
    // the complete line 1 (`echo one` lands on stdout).
    let (stderr, code) = rubash_stdin("echo one\necho ${x\n\"<x\n");
    assert_eq!(
        stderr,
        "bash: line 3: unexpected EOF while looking for matching `\"'\n"
    );
    assert_eq!(code, Some(2));
}

#[test]
fn c6_process_substitution_eof_reports_the_eof_line() {
    // `v=(<(echo`: the `<(` body's comsub route reports `)' at the EOF line,
    // rc 1 via the array-list family (GNU probe p2). A plain array list
    // keeps the open-line report (probe p4).
    let (stderr, code) = rubash_stdin("v=(<(echo\n");
    assert_eq!(
        stderr,
        "bash: line 2: unexpected EOF while looking for matching `)'\n"
    );
    assert_eq!(code, Some(1));

    let (stderr, code) = rubash_stdin("v=(a\n");
    assert_eq!(
        stderr,
        "bash: line 1: unexpected EOF while looking for matching `)'\n"
    );
    assert_eq!(code, Some(1));

    // A body spanning one more line reports that line (GNU probe p6: line 3).
    let (stderr, _) = rubash_stdin("v=(<(echo\necho x\n");
    assert_eq!(
        stderr,
        "bash: line 3: unexpected EOF while looking for matching `)'\n"
    );
}

#[test]
fn c8_write_error_names_the_builtin_with_the_errno_text() {
    // Read-only fd (here fd 0 dup'd for the read side): the dup succeeds,
    // the write fails at sh_chkwrite — `<builtin>: write error: Bad file
    // descriptor`, rc 1 (GNU probe c8h).
    let (stderr, code) = rubash_stdin("exec 3<&0\necho x >&3\n");
    assert_eq!(
        stderr,
        "bash: line 2: echo: write error: Bad file descriptor\n"
    );
    assert_eq!(code, Some(1));

    // The wording is builtin-owned and stable across echo/printf and the
    // trailing-newline forms (huck#190's drift is gone; GNU probes
    // c8e/c8f/c8g).
    for script in [
        "exec 3<&0\nprintf \"abc\" >&3\n",
        "exec 3<&0\nprintf \"abc\\n\" >&3\n",
        "exec 3<&0\necho -n abc >&3\n",
    ] {
        let (stderr, code) = rubash_stdin(script);
        let expected = if script.contains("printf") {
            "bash: line 2: printf: write error: Bad file descriptor\n"
        } else {
            "bash: line 2: echo: write error: Bad file descriptor\n"
        };
        assert_eq!(stderr, expected, "{script}");
        assert_eq!(code, Some(1), "{script}");
    }

    // A dup of a CLOSED fd still fails at setup with the fd-number form
    // (redir.c EBADF at do_redirections; cli_tests.rs c_command shape).
    let (stderr, code) = rubash_stdin("exec 3>&1\nexec 3>&-\necho fail >&3\n");
    assert_eq!(
        stderr, "bash: line 3: 3: Bad file descriptor\n",
        "closed-fd dup stays a setup failure"
    );
    assert_eq!(code, Some(1));
}
