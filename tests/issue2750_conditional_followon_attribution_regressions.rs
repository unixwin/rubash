//! rubash#2750 — the `[[ ]]` follow-on: when a conditional's own
//! diagnostic fires, GNU's report_syntax_error names the token the
//! READER stopped at, computed by walking the input text
//! (parse.y error_token_from_text): after a complete-but-invalid
//! conditional that is the token glued to the `]]` (`;`, `&&`, `|` —
//! a doubled operator contributes its first character, a redirect
//! only its first byte), and after an operator fragment carrying the
//! brackets (`==]]`) it is likewise the glued follow-on. A following
//! WORD is not walked into: the `]]` itself is named, and the first
//! error always stands — a later token (a stray `;;`, a word) never
//! replaces the conditional's own report.
//!
//! `]]` closes only as an exact standalone token (parse.y
//! special_case_tokens): a word carrying it (`x]]`) is an ordinary
//! argument, so `[[ x]] echo hi` fails at `echo` inside the still
//! open conditional.
//!
//! Expectations measured against GNU bash 5.3.20 (2026-11-04).

use std::path::PathBuf;
use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_file(tag: &str, script: &str) -> Run {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash2750-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    std::fs::write(dir.join("case.sh"), script).expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("case.sh")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &dir)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn run_c(script: &str) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn assert_run(tag: &str, run: &Run, stdout: &str, stderr: &str, code: i32) {
    assert_eq!(run.stdout, stdout, "{tag}: stdout");
    assert_eq!(run.stderr, stderr, "{tag}: stderr");
    assert_eq!(run.code, Some(code), "{tag}: rc");
}

const MSG_ARG: &str = "unexpected argument `]]' to conditional binary operator";

#[test]
fn issue2750_glued_semicolon_follower() {
    let run = run_c("[[ a == ]]; echo hi");
    assert_run(
        "semi",
        &run,
        "",
        &format!(
            "bash: -c: line 1: {MSG_ARG}\n\
             bash: -c: line 1: syntax error near `;'\n\
             bash: -c: line 1: `[[ a == ]]; echo hi'\n"
        ),
        2,
    );
}

#[test]
fn issue2750_glued_andand_follower_names_single_ampersand() {
    let run = run_c("[[ a == ]]&& echo hi");
    assert_run(
        "andand",
        &run,
        "",
        &format!(
            "bash: -c: line 1: {MSG_ARG}\n\
             bash: -c: line 1: syntax error near `&'\n\
             bash: -c: line 1: `[[ a == ]]&& echo hi'\n"
        ),
        2,
    );
}

#[test]
fn issue2750_glued_pipe_follower_after_unary() {
    let run = run_c("[[ -v ]]| cat");
    assert_run(
        "pipe",
        &run,
        "",
        "bash: -c: line 1: unexpected argument `]]' to conditional unary operator\n\
         bash: -c: line 1: syntax error near `|'\n\
         bash: -c: line 1: `[[ -v ]]| cat'\n",
        2,
    );
}

#[test]
fn issue2750_case_terminator_follower_keeps_cond_error() {
    // The stray `;;` must not replace the conditional's own report.
    let run = run_c("[[ a == ]];;");
    assert_run(
        "dsemi",
        &run,
        "",
        &format!(
            "bash: -c: line 1: {MSG_ARG}\n\
             bash: -c: line 1: syntax error near `;'\n\
             bash: -c: line 1: `[[ a == ]];;'\n"
        ),
        2,
    );
}

#[test]
fn issue2750_glued_rparen_follower_walks_over_paren() {
    let run = run_c("[[ a == ]])");
    assert_run(
        "rparen",
        &run,
        "",
        &format!(
            "bash: -c: line 1: {MSG_ARG}\n\
             bash: -c: line 1: syntax error near `]])'\n\
             bash: -c: line 1: `[[ a == ]])'\n"
        ),
        2,
    );
}

#[test]
fn issue2750_word_follower_names_the_closer_itself() {
    let run = run_c("[[ a == ]] echo hi");
    assert_run(
        "word",
        &run,
        "",
        &format!(
            "bash: -c: line 1: {MSG_ARG}\n\
             bash: -c: line 1: syntax error near `]]'\n\
             bash: -c: line 1: `[[ a == ]] echo hi'\n"
        ),
        2,
    );
}

#[test]
fn issue2750_no_follower_names_the_closer_itself() {
    let run = run_c("[[ a == ]]");
    assert_run(
        "none",
        &run,
        "",
        &format!(
            "bash: -c: line 1: {MSG_ARG}\n\
             bash: -c: line 1: syntax error near `]]'\n\
             bash: -c: line 1: `[[ a == ]]'\n"
        ),
        2,
    );
}

#[test]
fn issue2750_andand_fragment_after_and_names_semicolon() {
    let run = run_c("[[ a &&]]; echo hi");
    assert_run(
        "e",
        &run,
        "",
        "bash: -c: line 1: syntax error near `;'\n\
         bash: -c: line 1: `[[ a &&]]; echo hi'\n",
        2,
    );
}

#[test]
fn issue2750_operator_fragment_background_follower() {
    let run = run_c("[[ a ==]]&");
    assert_run(
        "op-bg",
        &run,
        "",
        "bash: -c: line 1: unexpected token `==]]', conditional binary operator expected\n\
         bash: -c: line 1: syntax error near `&'\n\
         bash: -c: line 1: `[[ a ==]]&'\n",
        2,
    );
}

#[test]
fn issue2750_operator_fragment_andand_follower() {
    let run = run_c("[[ a ==]]&& echo hi");
    assert_run(
        "op-andand",
        &run,
        "",
        "bash: -c: line 1: unexpected token `==]]', conditional binary operator expected\n\
         bash: -c: line 1: syntax error near `&'\n\
         bash: -c: line 1: `[[ a ==]]&& echo hi'\n",
        2,
    );
}

#[test]
fn issue2750_operator_fragment_redirect_err_follower() {
    // A redirect follower contributes only its first byte.
    let run = run_c("[[ a ==]]>&f");
    assert_run(
        "op-redir",
        &run,
        "",
        "bash: -c: line 1: unexpected token `==]]', conditional binary operator expected\n\
         bash: -c: line 1: syntax error near `==]]>'\n\
         bash: -c: line 1: `[[ a ==]]>&f'\n",
        2,
    );
}

#[test]
fn issue2750_operator_fragment_herestring_follower() {
    let run = run_c("[[ a ==]]<<<x");
    assert_run(
        "op-herestring",
        &run,
        "",
        "bash: -c: line 1: unexpected token `==]]', conditional binary operator expected\n\
         bash: -c: line 1: syntax error near `==]]<'\n\
         bash: -c: line 1: `[[ a ==]]<<<x'\n",
        2,
    );
}

#[test]
fn issue2750_leading_andand_names_single_ampersand() {
    let run = run_c("[[ && a ]]; echo hi");
    assert_run(
        "idx-and",
        &run,
        "",
        "bash: -c: line 1: unexpected token `&&' in conditional command\n\
         bash: -c: line 1: syntax error near `&'\n\
         bash: -c: line 1: `[[ && a ]]; echo hi'\n",
        2,
    );
}

#[test]
fn issue2750_leading_oror_names_single_pipe() {
    let run = run_c("[[ || a ]]; echo hi");
    assert_run(
        "idx-or",
        &run,
        "",
        "bash: -c: line 1: unexpected token `||' in conditional command\n\
         bash: -c: line 1: syntax error near `|'\n\
         bash: -c: line 1: `[[ || a ]]; echo hi'\n",
        2,
    );
}

#[test]
fn issue2750_mid_andand_names_single_ampersand() {
    let run = run_c("[[ a && && b ]]; echo hi");
    assert_run(
        "idx-mid",
        &run,
        "",
        "bash: -c: line 1: unexpected token `&&' in conditional command\n\
         bash: -c: line 1: syntax error near `&'\n\
         bash: -c: line 1: `[[ a && && b ]]; echo hi'\n",
        2,
    );
}

#[test]
fn issue2750_newline_follower_is_not_glued() {
    // A newline after the `]]` is a real separator: the follow-on
    // walk must not invent a glued `;` from it.
    let run = run_file("j", "[[ a == ]]\n&& echo x\n");
    assert_run(
        "j",
        &run,
        "",
        &format!(
            "case.sh: line 1: {MSG_ARG}\n\
             case.sh: line 1: syntax error near `]]'\n\
             case.sh: line 1: `[[ a == ]]'\n"
        ),
        2,
    );
}

#[test]
fn issue2750_glued_word_is_not_a_closer() {
    let run = run_c("[[ x]] echo hi");
    assert_run(
        "glued-word",
        &run,
        "",
        "bash: -c: line 1: unexpected token `echo', conditional binary operator expected\n\
         bash: -c: line 1: syntax error near `echo'\n\
         bash: -c: line 1: `[[ x]] echo hi'\n",
        2,
    );
}

#[test]
fn issue2750_glued_word_then_separator() {
    let run = run_c("[[ x]]; echo rc=$?");
    assert_run(
        "glued-word-semi",
        &run,
        "",
        "bash: -c: line 1: unexpected token `;', conditional binary operator expected\n\
         bash: -c: line 1: syntax error near `;'\n\
         bash: -c: line 1: `[[ x]]; echo rc=$?'\n",
        2,
    );
}

#[test]
fn issue2750_glued_rhs_word_completes_the_comparison() {
    let run = run_c("[[ a == x]]; echo rc=$?");
    assert_run(
        "glued-rhs",
        &run,
        "",
        "bash: -c: line 1: syntax error in conditional expression: unexpected token `;'\n\
         bash: -c: line 1: syntax error near `;'\n\
         bash: -c: line 1: `[[ a == x]]; echo rc=$?'\n",
        2,
    );
}

#[test]
fn issue2750_valid_cond_separatorless_word_is_plain_near() {
    // Control: a VALID conditional followed by a separator-less word
    // is the ordinary yyerror on that word.
    let run = run_c("[[ x ]] echo hi");
    assert_run(
        "valid-word",
        &run,
        "",
        "bash: -c: line 1: syntax error near unexpected token `echo'\n\
         bash: -c: line 1: `[[ x ]] echo hi'\n",
        2,
    );
}
