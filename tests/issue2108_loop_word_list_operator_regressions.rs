//! rubash#2108 — the `for`/`select` word list admits WORD tokens only;
//! an operator token inside the list is the syntax-error token at its
//! own position, and the name slot blames a non-word token itself
//! instead of a later `do`.
//!
//! GNU: `word_list: WORD | word_list WORD` (parse.y:526-529) is the
//! only list production — parse.y:921 (`FOR WORD newline_list IN
//! word_list list_terminator newline_list DO ...`) and parse.y:1007
//! (the SELECT twin) therefore reject every operator token where it
//! stands, and yyerror discards the whole command. The name slot is
//! `FOR WORD ...` (parse.y:893) / `SELECT WORD ...` (parse.y:979):
//! exactly one WORD may follow the keyword. Rubash's hand-rolled head
//! parsers stepped over operator tokens in the list (the collect
//! returns None for the whole class and the loop skipped it), so the
//! operator vanished, its target was collected as a list word, and a
//! bogus loop RAN (rc 0); and a non-word in the name slot fell through
//! to the word path, which walked on to `do` and blamed IT.
//!
//! Quoted or escaped operator spellings are WORD text, never operator
//! tokens (read_token_word), so `'|'`, `'>'`, `';;'`, `\;` remain
//! ordinary list words — the control pins below pin that side too.
//! All expectations measured against GNU bash 5.3.20 with script
//! files (2026-10-29).

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run(tag: &str, script: &str) -> Run {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash2108-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    std::fs::write(dir.join("case.sh"), script).expect("write fixture");
    std::fs::write(dir.join("in.txt"), "indata\n").expect("write input fixture");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("case.sh")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run rubash");
    // `select` controls read their answer from stdin.
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b"1\n")
        .expect("feed stdin");
    let output = child.wait_with_output().expect("wait rubash");
    let _ = std::fs::remove_dir_all(&dir);
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

/// A broken head: GNU's single syntax error, rc 2, and NOTHING executes
/// (empty stdout — no bogus iterations, no AFTER).
fn assert_gnu_error(tag: &str, script: &str, expected_stderr: &str) {
    let run = run(tag, script);
    assert_eq!(
        run.stdout, "",
        "{tag}: stdout must stay empty: {:?}",
        run.stdout
    );
    assert_eq!(run.stderr, expected_stderr, "{tag}: stderr");
    assert_eq!(run.code, Some(2), "{tag}: rc");
}

/// A well-formed head: the loop runs, rc 0, stderr as given.
fn assert_runs(tag: &str, script: &str, expected_stdout: &str, expected_stderr: &str) {
    let run = run(tag, script);
    assert_eq!(run.stdout, expected_stdout, "{tag}: stdout");
    assert_eq!(run.stderr, expected_stderr, "{tag}: stderr");
    assert_eq!(run.code, Some(0), "{tag}: rc");
}

// ---------------------------------------------------------------------
// Face B — operator tokens inside the `for` word list (parse.y:526-529)
// ---------------------------------------------------------------------

#[test]
fn issue2108_for_list_pipe_is_error_token() {
    assert_gnu_error(
        "for-pipe",
        "for i in 1 2 | cat; do echo \"iter $i\"; done\necho AFTER\n",
        "case.sh: line 1: syntax error near unexpected token `|'\n\
         case.sh: line 1: `for i in 1 2 | cat; do echo \"iter $i\"; done'\n",
    );
}

#[test]
fn issue2108_for_list_redirect_out_is_error_token() {
    assert_gnu_error(
        "for-redir",
        "for i in > f 1; do echo \"iter $i\"; done\necho AFTER\n",
        "case.sh: line 1: syntax error near unexpected token `>'\n\
         case.sh: line 1: `for i in > f 1; do echo \"iter $i\"; done'\n",
    );
}

#[test]
fn issue2108_for_list_and_is_error_token() {
    assert_gnu_error(
        "for-and",
        "for i in 1 && cat; do echo \"iter $i\"; done\necho AFTER\n",
        "case.sh: line 1: syntax error near unexpected token `&&'\n\
         case.sh: line 1: `for i in 1 && cat; do echo \"iter $i\"; done'\n",
    );
}

#[test]
fn issue2108_for_list_or_is_error_token() {
    assert_gnu_error(
        "for-or",
        "for i in 1 || cat; do echo \"iter $i\"; done\necho AFTER\n",
        "case.sh: line 1: syntax error near unexpected token `||'\n\
         case.sh: line 1: `for i in 1 || cat; do echo \"iter $i\"; done'\n",
    );
}

#[test]
fn issue2108_for_list_pipe_err_is_error_token() {
    assert_gnu_error(
        "for-pipeerr",
        "for i in 1 |& cat; do echo \"iter $i\"; done\necho AFTER\n",
        "case.sh: line 1: syntax error near unexpected token `|&'\n\
         case.sh: line 1: `for i in 1 |& cat; do echo \"iter $i\"; done'\n",
    );
}

#[test]
fn issue2108_for_list_numbered_redirect_blames_the_number() {
    // GNU's lexer returns the digit run as its own IO_NUMBER token, so
    // the blame is `2', not the fused `2>' spelling.
    assert_gnu_error(
        "for-redirerr",
        "for i in 1 2>err; do echo \"iter $i\"; done\necho AFTER\n",
        "case.sh: line 1: syntax error near unexpected token `2'\n\
         case.sh: line 1: `for i in 1 2>err; do echo \"iter $i\"; done'\n",
    );
}

#[test]
fn issue2108_for_list_redirect_in_is_error_token() {
    assert_gnu_error(
        "for-redirin",
        "for i in 1 < in.txt; do echo \"iter $i\"; done\necho AFTER\n",
        "case.sh: line 1: syntax error near unexpected token `<'\n\
         case.sh: line 1: `for i in 1 < in.txt; do echo \"iter $i\"; done'\n",
    );
}

#[test]
fn issue2108_for_list_append_is_error_token() {
    assert_gnu_error(
        "for-append",
        "for i in 1 >> log; do echo \"iter $i\"; done\necho AFTER\n",
        "case.sh: line 1: syntax error near unexpected token `>>'\n\
         case.sh: line 1: `for i in 1 >> log; do echo \"iter $i\"; done'\n",
    );
}

#[test]
fn issue2108_for_list_background_is_error_token() {
    assert_gnu_error(
        "for-bg",
        "for i in 1 & do echo \"iter $i\"; done\necho AFTER\n",
        "case.sh: line 1: syntax error near unexpected token `&'\n\
         case.sh: line 1: `for i in 1 & do echo \"iter $i\"; done'\n",
    );
}

#[test]
fn issue2108_for_list_heredoc_is_error_token() {
    assert_gnu_error(
        "for-heredoc",
        "for i in 1 <<EOF; do echo \"iter $i\"; done\nbody\nEOF\necho AFTER\n",
        "case.sh: line 1: syntax error near unexpected token `<<'\n\
         case.sh: line 1: `for i in 1 <<EOF; do echo \"iter $i\"; done'\n",
    );
}

#[test]
fn issue2108_for_list_herestring_is_error_token() {
    assert_gnu_error(
        "for-herestring",
        "for i in 1 <<< word; do echo \"iter $i\"; done\necho AFTER\n",
        "case.sh: line 1: syntax error near unexpected token `<<<'\n\
         case.sh: line 1: `for i in 1 <<< word; do echo \"iter $i\"; done'\n",
    );
}

// ---------------------------------------------------------------------
// Face B — operator tokens inside the `select` word list (parse.y:1007)
// ---------------------------------------------------------------------

#[test]
fn issue2108_select_list_pipe_is_error_token() {
    assert_gnu_error(
        "select-pipe",
        "select s in a b | cat; do echo \"sel $s\"; done\necho AFTER\n",
        "case.sh: line 1: syntax error near unexpected token `|'\n\
         case.sh: line 1: `select s in a b | cat; do echo \"sel $s\"; done'\n",
    );
}

#[test]
fn issue2108_select_list_redirect_out_is_error_token() {
    assert_gnu_error(
        "select-redir",
        "select s in a > f; do echo \"sel $s\"; done\necho AFTER\n",
        "case.sh: line 1: syntax error near unexpected token `>'\n\
         case.sh: line 1: `select s in a > f; do echo \"sel $s\"; done'\n",
    );
}

#[test]
fn issue2108_select_list_and_is_error_token() {
    assert_gnu_error(
        "select-and",
        "select s in a && cat; do echo \"sel $s\"; done\necho AFTER\n",
        "case.sh: line 1: syntax error near unexpected token `&&'\n\
         case.sh: line 1: `select s in a && cat; do echo \"sel $s\"; done'\n",
    );
}

// ---------------------------------------------------------------------
// Face A — the name slot blames its own token (parse.y:893 / 979)
// ---------------------------------------------------------------------

#[test]
fn issue2108_for_name_slot_semicolon_blames_semicolon() {
    assert_gnu_error(
        "for-name-semi",
        "for; do echo x; done\necho AFTER\n",
        "case.sh: line 1: syntax error near unexpected token `;'\n\
         case.sh: line 1: `for; do echo x; done'\n",
    );
}

#[test]
fn issue2108_for_name_slot_redirect_blames_redirect() {
    assert_gnu_error(
        "for-name-redir",
        "for > f; do echo x; done\necho AFTER\n",
        "case.sh: line 1: syntax error near unexpected token `>'\n\
         case.sh: line 1: `for > f; do echo x; done'\n",
    );
}

#[test]
fn issue2108_for_name_slot_pipe_blames_pipe() {
    assert_gnu_error(
        "for-name-pipe",
        "for | cat; do echo x; done\necho AFTER\n",
        "case.sh: line 1: syntax error near unexpected token `|'\n\
         case.sh: line 1: `for | cat; do echo x; done'\n",
    );
}

#[test]
fn issue2108_select_name_slot_semicolon_blames_semicolon() {
    assert_gnu_error(
        "select-name-semi",
        "select; do echo x; done\necho AFTER\n",
        "case.sh: line 1: syntax error near unexpected token `;'\n\
         case.sh: line 1: `select; do echo x; done'\n",
    );
}

#[test]
fn issue2108_select_name_slot_redirect_blames_redirect() {
    assert_gnu_error(
        "select-name-redir",
        "select > f; do echo x; done\necho AFTER\n",
        "case.sh: line 1: syntax error near unexpected token `>'\n\
         case.sh: line 1: `select > f; do echo x; done'\n",
    );
}

// ---------------------------------------------------------------------
// Controls — quoted/escaped operators and ordinary words still iterate
// ---------------------------------------------------------------------

#[test]
fn issue2108_quoted_operators_are_list_words() {
    assert_runs(
        "quoted-ops",
        "for i in '|' '>' '&&' ';;' ';&'; do echo \"iter [$i]\"; done\n",
        "iter [|]\niter [>]\niter [&&]\niter [;;]\niter [;&]\n",
        "",
    );
}

#[test]
fn issue2108_escaped_operators_are_list_words() {
    assert_runs(
        "escaped-ops",
        "for i in \\; \\> a; do echo \"iter [$i]\"; done\n",
        "iter [;]\niter [>]\niter [a]\n",
        "",
    );
}

#[test]
fn issue2108_plain_list_still_iterates() {
    assert_runs(
        "basic",
        "for i in 1 2 3; do echo \"iter $i\"; done\n",
        "iter 1\niter 2\niter 3\n",
        "",
    );
}

#[test]
fn issue2108_empty_in_list_still_works() {
    assert_runs(
        "empty-in",
        "for i in; do echo x; done\necho EMPTY-OK\n",
        "EMPTY-OK\n",
        "",
    );
}

#[test]
fn issue2108_keyword_spellings_are_list_words() {
    assert_runs(
        "keyword-words",
        "for i in do done in; do echo \"iter $i\"; done\n",
        "iter do\niter done\niter in\n",
        "",
    );
}

#[test]
fn issue2108_brace_and_substitution_words_still_work() {
    assert_runs(
        "brace-comsub",
        "for i in {1..3}; do echo \"iter $i\"; done\nfor j in a=b $(echo z); do echo \"iter $j\"; done\n",
        "iter 1\niter 2\niter 3\niter a=b\niter z\n",
        "",
    );
}

#[test]
fn issue2108_default_positional_list_still_works() {
    assert_runs(
        "default-pos",
        "set -- p q\nfor i; do echo \"iter $i\"; done\n",
        "iter p\niter q\n",
        "",
    );
}

#[test]
fn issue2108_arithmetic_for_still_works() {
    assert_runs(
        "arith",
        "for ((k=0;k<2;k++)); do echo \"n $k\"; done\n",
        "n 0\nn 1\n",
        "",
    );
}

#[test]
fn issue2108_process_substitution_words_still_work() {
    // A process substitution begins with an operator-shaped token but
    // is word text in GNU's lexer; the collect takes it before the
    // operator gate may judge the token.
    assert_runs(
        "procsubst",
        "for p in <(printf hello); do cat \"$p\"; done\n",
        "hello",
        "",
    );
}

#[test]
fn issue2108_select_wellformed_still_works() {
    assert_runs(
        "select-ok",
        "select s in a b; do echo \"sel $s\"; break; done\n",
        "sel a\n",
        "1) a\n2) b\n#? ",
    );
}
