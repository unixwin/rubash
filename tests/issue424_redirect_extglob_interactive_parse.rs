//! Issue rubash#424 (wt63 lane): three shapes from the wt56 audit.
//!
//! 1. Redirect-target pathname expansion follows GNU's
//!    expand_words_no_vars contract (subst.c:12590-12700, reached from
//!    redir.c:298 redirection_expand): a no-match pattern is DEQUOTED and
//!    kept as the literal filename (subst.c:12645-12649 dequote_string,
//!    subst.c:12676 "failed glob expressions are left unchanged"), and
//!    failglob reports `no match: WORD' and DISCARDs the command list with
//!    status 1 (subst.c:12663-12668). An extglob introducer preceded by a
//!    backslash never opens a pattern group (parse.y:5366-5375 backslash
//!    branch consumes `\c' before the parse.y:5466 PATTERN_CHAR rule), so
//!    `>\@(zz)x' is `syntax error near unexpected token `(''.
//! 2. An interactive shell survives a parse error (GNU eval.c:202-205:
//!    read_command failure sets EOF_Reached only `if (interactive == 0)')
//!    and prints no offending-line echo in interactive mode
//!    (parse.y:6865/6886 `if (interactive == 0) print_offending_line').
//! 3. The offending-line echo is the WHOLE physical input line
//!    (parse.y:6814-6826 print_offending_line: shell_input_line verbatim),
//!    including text outside the conditional's token slice — the
//!    `f() { ...; }' wrapper of a function-definition body.

use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};

fn fixture(tag: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new(env!("CARGO_BIN_EXE_rubash"))
        .ancestors()
        .nth(2)
        .unwrap()
        .join("probe424-tests")
        .join(tag);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn rubash_script(dir: &std::path::Path, script: &str) -> (String, String, Option<i32>) {
    let file = dir.join("case.sh");
    fs::write(&file, script.replace('\r', "")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(dir)
        .arg("case.sh")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

// -------------------------------------------------------------------------
// Shape 1: redirect-target pathname expansion
// -------------------------------------------------------------------------

/// subst.c:12645-12676: an extglob pattern with no match is kept as the
/// (dequoted) literal filename; the redirect opens it (GNU creates the
/// literal file, RC=0). `@(zz)at` is a Windows-legal name.
#[test]
fn extglob_redirect_no_match_keeps_dequoted_literal() {
    let dir = fixture("extglob-literal");
    let (stdout, stderr, code) = rubash_script(
        &dir,
        "shopt -s extglob\necho one >@(zz)at\nprintf 'RC=%s\\n' $?\nif [ -f '@(zz)at' ]; then echo LITERAL; fi\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "RC=0\nLITERAL\n", "stderr: {stderr}");
}

/// The retained literal is the QUOTE-REMOVED word (subst.c:12645-12649):
/// `>"@(zz)q"` opens `@(zz)q` — never a name led by the CTLESC carrier
/// byte (rubash#424: the 0x11 marker reached open() and Windows rejected
/// it with ERROR_INVALID_NAME).
#[test]
fn quoted_extglob_redirect_target_opens_quote_removed_name() {
    let dir = fixture("extglob-quoted");
    let (stdout, stderr, code) = rubash_script(
        &dir,
        "shopt -s extglob\necho b >\"@(zz)q\"\nprintf 'RC=%s\\n' $?\nif [ -f '@(zz)q' ]; then echo LITERAL; fi\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "RC=0\nLITERAL\n", "stderr: {stderr}");
}

/// subst.c:12663-12668: failglob fires inside redirection_expand too —
/// `no match: @(zz)at', status 1, nothing opens, the rest of the LINE is
/// discarded and the next line runs.
#[test]
fn failglob_redirect_target_aborts_command_list() {
    let dir = fixture("extglob-failglob");
    let (stdout, stderr, code) = rubash_script(
        &dir,
        "shopt -s extglob failglob\necho four >@(zz)at; echo SAME-LINE\necho RC=$? NEXT\nprintf 'after=%s\\n' $?\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stderr.contains("no match: @(zz)at"), "stderr: {stderr}");
    assert!(!stdout.contains("SAME-LINE"), "stdout: {stdout}");
    assert!(stdout.contains("NEXT\n"), "stdout: {stdout}");
    assert!(!dir.join("@(zz)at").exists(), "failed target must not open");
}

/// parse.y:5466 with the gate closed (extglob off at parse time): the `('
/// ends the redirect-target word — `syntax error near unexpected token `(''
/// and the script aborts with status 2.
#[test]
fn extglob_redirect_without_shopt_is_syntax_error() {
    let dir = fixture("extglob-noshopt");
    let (stdout, stderr, code) = rubash_script(&dir, "echo one >?(zz)out\necho NEVER\n");
    assert_eq!(code, Some(2), "stderr: {stderr}");
    assert!(
        stderr.contains("syntax error near unexpected token `('"),
        "stderr: {stderr}"
    );
    assert_eq!(stdout, "", "stdout: {stdout}");
}

/// parse.y:5366-5375: the backslash pair consumes `\@' before the
/// EXTENDED_GLOB PATTERN_CHAR rule (parse.y:5466) can see it, so the `('
/// ends the word even WITH extglob on — same syntax error (verified vs
/// WSL GNU Bash 5.3.0: `echo e >\@(zz)esc').
#[test]
fn escaped_extglob_introducer_is_syntax_error() {
    let dir = fixture("extglob-escaped");
    let (stdout, stderr, code) =
        rubash_script(&dir, "shopt -s extglob\necho e >\\@(zz)esc\necho NEVER\n");
    assert_eq!(code, Some(2), "stderr: {stderr}");
    assert!(
        stderr.contains("syntax error near unexpected token `('"),
        "stderr: {stderr}"
    );
    assert_eq!(stdout, "", "stdout: {stdout}");
}

/// The issue-title form, STANDALONE: a redirect with no command word.
/// The gate is closed (same-line `shopt' never arms the parse gate and
/// extglob is off by default), so the `(' strands out of the target word
/// and yacc reports `syntax error near unexpected token `(''; rc=2 and
/// NO literal `?(zz)out' file is created.
#[test]
fn standalone_redirect_target_is_syntax_error_without_literal_file() {
    let dir = fixture("extglob-standalone");
    let (stdout, stderr, code) = rubash_script(&dir, ">?(zz)out\necho NEVER\n");
    assert_eq!(code, Some(2), "stderr: {stderr}");
    assert!(
        stderr.contains("syntax error near unexpected token `('"),
        "stderr: {stderr}"
    );
    assert_eq!(stdout, "", "stdout: {stdout}");
    assert!(!dir.join("?(zz)out").exists(), "no literal file may appear");
}

/// The canonical issue repro as one line: `echo hi >?(zz)out' — rc=2,
/// the GNU diagnostic, no file, and nothing after the abort runs.
#[test]
fn canonical_issue_repro_rejects_without_creating_file() {
    let dir = fixture("extglob-canonical");
    let (stdout, stderr, code) = rubash_script(&dir, "echo hi >?(zz)out\necho NEVER\n");
    assert_eq!(code, Some(2), "stderr: {stderr}");
    assert!(
        stderr.contains("syntax error near unexpected token `('"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("`echo hi >?(zz)out'"),
        "offending line echoed; stderr: {stderr}"
    );
    assert_eq!(stdout, "", "stdout: {stdout}");
    assert!(!dir.join("?(zz)out").exists(), "no literal file may appear");
}

/// The rejection covers the whole redirect-operator family: `>>' and
/// `2>' take the same WORD-operand grammar (parse.y:532-575), so their
/// extglob targets strand the `(' identically.
#[test]
fn append_and_err_redirect_extglob_targets_reject() {
    let dir = fixture("extglob-op-family");
    for script in ["echo a >>?(zz)out", "echo a 2>?(zz)out"] {
        let (stdout, stderr, code) = rubash_script(&dir, &format!("{script}\necho NEVER\n"));
        assert_eq!(code, Some(2), "script: {script}; stderr: {stderr}");
        assert!(
            stderr.contains("syntax error near unexpected token `('"),
            "script: {script}; stderr: {stderr}"
        );
        assert_eq!(stdout, "", "script: {script}; stdout: {stdout}");
        assert!(
            !dir.join("?(zz)out").exists(),
            "script: {script}; no literal file may appear"
        );
    }
}

/// The INPUT side is NOT part of the rejection: `<(' is process
/// substitution (parse.y reserved word, gram.y `procsub'), both with the
/// gate off and on — `echo hi <(zz)' runs zz and reads the pipe
/// (verified GNU: `zz: command not found' on stderr, `hi /dev/fd/63').
#[test]
fn input_side_process_substitution_still_works() {
    for prologue in ["", "shopt -s extglob\n"] {
        let dir = fixture("procsub-input-side");
        let (stdout, stderr, code) = rubash_script(&dir, &format!("{prologue}echo hi <(zz)\n"));
        assert_eq!(code, Some(0), "stderr: {stderr}");
        assert!(stderr.contains("zz: command not found"), "stderr: {stderr}");
        assert!(
            stdout.contains("hi /dev/fd/"),
            "stdout: {stdout}; stderr: {stderr}"
        );
    }
}

// -------------------------------------------------------------------------
// Shape 2: interactive session survives a parse error
// -------------------------------------------------------------------------

/// eval.c:202-205: an interactive shell reports the parse error and
/// PROMPTS AGAIN; the session keeps running (`DEF-OK', `c12' -> 127) and
/// ends with the last command's status. The ExitCode(2) of the parse
/// error is the DISCARD classification, not an exit-shell jump.
#[test]
fn interactive_session_survives_parse_error() {
    let dir = fixture("interactive-parse");
    let script = "c12() { [[ ((zz)) == zz ]] && echo C12-TRUE || echo C12-FALSE; }\necho DEF-OK\nc12\necho AFTER=$?\n";
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("-i")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash -i");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(script.as_bytes())
        .unwrap();
    let output = child.wait_with_output().expect("wait rubash -i");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stdout.contains("DEF-OK"),
        "session must continue after the parse error; stdout: {stdout}; stderr: {stderr}"
    );
    // `c12' was never defined (the definition died at the parse error), so
    // it fails with 127 and `$?' keeps that status (GNU: c12 -> 127).
    assert!(
        stdout.contains("AFTER=127"),
        "stdout: {stdout}; stderr: {stderr}"
    );
    assert_eq!(output.status.code(), Some(0), "stderr: {stderr}");
}

/// parse.y:6865/6886: `print_offending_line' runs only `if (interactive ==
/// 0)' — the interactive parse-error report is the two diagnostic lines,
/// with NO third `` `line' `` echo (byte-verified vs WSL GNU Bash 5.3.0).
#[test]
fn interactive_parse_error_prints_no_offending_line_echo() {
    let dir = fixture("interactive-echo");
    let script = "c12() { [[ ((zz)) == zz ]] && echo T; }\necho OK\n";
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("-i")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash -i");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(script.as_bytes())
        .unwrap();
    let output = child.wait_with_output().expect("wait rubash -i");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.contains("syntax error in conditional expression: unexpected token `=='"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("syntax error near `=='"),
        "stderr: {stderr}"
    );
    assert!(
        !stderr.contains("`[[ ((zz)) == zz ]]"),
        "interactive report must not echo the offending line; stderr: {stderr}"
    );
}

// -------------------------------------------------------------------------
// Shape 3: offending-line echo is the whole physical line
// -------------------------------------------------------------------------

/// parse.y:6814-6826: the echo is shell_input_line verbatim — for a
/// function-definition body error that includes the `name() { ' prefix and
/// the closing `}' (the body tokens alone cannot reconstruct it).
#[test]
fn conditional_error_echoes_whole_function_definition_line() {
    let dir = fixture("echo-whole-line");
    let (stdout, stderr, code) = rubash_script(
        &dir,
        "f2() { [[ ((zz)) == zz ]] && echo YES2; }\necho AFTER\n",
    );
    assert_eq!(code, Some(2), "stderr: {stderr}");
    assert_eq!(stdout, "", "stdout: {stdout}");
    assert!(
        stderr.contains("`f2() { [[ ((zz)) == zz ]] && echo YES2; }'"),
        "stderr: {stderr}"
    );
}

/// Same rule for inline sections: the brace-group wrapper, the subshell
/// parens and the `while ...; do ... done' head all ride the echoed line.
#[test]
fn conditional_error_echoes_whole_compound_lines() {
    let dir = fixture("echo-whole-compound");
    for script in [
        "{ [[ ((zz)) == zz ]] && echo Y; }",
        "( [[ ((zz)) == zz ]] && echo Y )",
        "while false; do [[ ((zz)) == zz ]]; done",
    ] {
        let (stdout, stderr, code) = rubash_script(&dir, &format!("{script}\necho AFTER\n"));
        assert_eq!(code, Some(2), "script: {script}; stderr: {stderr}");
        assert_eq!(stdout, "", "script: {script}; stdout: {stdout}");
        assert!(
            stderr.contains(&format!("`{script}'")),
            "script: {script}; stderr: {stderr}"
        );
    }
}

/// A multi-line body echoes the error's OWN line verbatim, leading
/// whitespace included (parse.y:6814 strips only trailing newlines).
#[test]
fn conditional_error_echoes_own_indented_line() {
    let dir = fixture("echo-own-line");
    let (stdout, stderr, code) = rubash_script(
        &dir,
        "f3() {\n    [[ ((zz)) == zz ]] && echo YES3\n}\necho AFTER\n",
    );
    assert_eq!(code, Some(2), "stderr: {stderr}");
    assert_eq!(stdout, "", "stdout: {stdout}");
    assert!(
        stderr.contains("line 2: `    [[ ((zz)) == zz ]] && echo YES3'"),
        "stderr: {stderr}"
    );
}
