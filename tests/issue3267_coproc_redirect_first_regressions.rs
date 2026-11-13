//! rubash#3267 regressions: `coproc` rejected a redirect-first
//! simple-command body (`coproc >x foooo` was a syntax error, rc 2).
//!
//! GNU model (bash 5.3, parse.y:1125-1175): the coproc grammar is
//! `COPROC [WORD] shell_command` / `COPROC simple_command`, and a
//! simple_command is redirections + words in any order
//! (parse.y:1262 compound_list, parse.y:532-575 redirection
//! productions) — so `>x foooo`, `foooo >y bar`, `>x foooo >y`,
//! `2>err foooo`, `<<HD cat` and `{var}>x foooo` are all legal
//! coproc bodies. rubash's coproc parser only collected WORD tokens
//! for the simple body and let `finish_compound_command` eat the
//! leading redirect(s), stranding the first word with no separator
//! — the main loop then reported it as an unexpected token.
//!
//! Expectations verified byte-for-byte against GNU Bash 5.3.20
//! script-file probes (self-compiled oracle, LC_ALL=C.UTF-8).
//! The function-body form of the minimal cell lives in the #3209
//! nail (`coproc_first_reports_definition_line`) and turns green
//! with this fix; it is not duplicated here.
//!
//! Correction (takeover lane, GNU oracle 2026-11-13): `coproc NAME >x
//! foooo` does NOT take NAME as the coproc name — GNU runs NAME as the
//! command word (`NAME: command not found`, rc 0), because the COPROC
//! WORD name production only wins before a compound/shell_command, not
//! before a redirect-led simple_command. The named cell below pins the
//! oracle reading, not the first draft's guess.
//!
//! Same round: a `!` after the first word is a plain word once ANY
//! redirection precedes it (`coproc >x foo ! bar`, `coproc foo >y !
//! bar` — both rc 0); the #1652 second-word-`!` error fires only with
//! no redirection consumed yet (`coproc foo ! bar` stays rc 2).
//!
//! Deliberately NOT pinned: a redirect before a COMPOUND body
//! (`coproc >x { ...; }`, `coproc >x ( ... )`, `coproc >x if ...`)
//! — GNU rejects those too (rc 2), but rubash already rejected
//! them on the base (same rc, different blame token), so they are
//! locked only as rc-2 guards below, not as exact-blame cells.
//! GNU blames `}`, `(`, `do`, `then` there; rubash blames the
//! compound starter (`{`, `(`, `for`, `if`) — a known residual.

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue3267-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn check(script: &str, expected_stdout: &str, expected_stderr: &str, expected_code: i32) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(
        code,
        Some(expected_code),
        "script: {script:?}\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert_eq!(stdout, expected_stdout, "script: {script:?}");
    assert_eq!(stderr, expected_stderr, "script: {script:?}");
}

fn check_syntax_error_near(script: &str, token: &str) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(
        code,
        Some(2),
        "script: {script:?}\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stderr.contains(&format!("syntax error near unexpected token `{token}'")),
        "script: {script:?}\nexpected blame `{token}', stderr: {stderr}"
    );
}

const NOT_FOUND: &str = "case.sh: line 1: foooo: command not found\n";
const NAME_NOT_FOUND: &str = "case.sh: line 1: NAME: command not found\n";
const FOO_NOT_FOUND: &str = "case.sh: line 1: foo: command not found\n";

// -----------------------------------------------------------------
// Redirect-first simple bodies (GNU rc 0)
// -----------------------------------------------------------------

#[test]
fn issue3267_redirect_first_minimal() {
    check("coproc >x foooo\n", "", NOT_FOUND, 0);
}

#[test]
fn issue3267_redirect_first_spaced_target() {
    check("coproc > x foooo\n", "", NOT_FOUND, 0);
}

#[test]
fn issue3267_redirect_first_two_words() {
    check("coproc >x foooo bar baz\n", "", NOT_FOUND, 0);
}

#[test]
fn issue3267_intermixed_redirect_after_word() {
    check("coproc foooo >y bar\n", "", NOT_FOUND, 0);
}

#[test]
fn issue3267_redirect_first_then_trailing() {
    check("coproc >x foooo >y\n", "", NOT_FOUND, 0);
}

#[test]
fn issue3267_redirect_first_numbered_fd() {
    check("coproc 2>/dev/null foooo\n", "", "", 0);
}

#[test]
fn issue3267_redirect_first_devnull_target() {
    check("coproc >/dev/null foooo\n", "", NOT_FOUND, 0);
}

#[test]
fn issue3267_redirect_first_heredoc_body() {
    check("coproc <<HD cat\nhello\nHD\necho done\n", "done\n", "", 0);
}

#[test]
fn issue3267_redirect_first_bang_is_a_word() {
    check(
        "coproc >x ! echo hi\n",
        "",
        "case.sh: line 1: !: command not found\n",
        0,
    );
}

#[test]
fn issue3267_redirect_first_applies_to_child() {
    check(
        "coproc >out.txt echo hello\nwait\ncat out.txt\n",
        "hello\n",
        "",
        0,
    );
}

#[test]
fn issue3267_redirect_first_named() {
    // GNU runs NAME as the command word here (see header): the name
    // production needs a compound/shell_command after WORD, and a
    // redirect-led simple_command does not qualify.
    check("coproc NAME >x foooo\n", "", NAME_NOT_FOUND, 0);
}

#[test]
fn issue3267_bang_after_leading_redirect_is_word() {
    // A redirection before the first word takes the simple_command
    // branch, so `!` is a plain word, not the #1652 second-word error.
    check("coproc >x foo ! bar\n", "", FOO_NOT_FOUND, 0);
}

#[test]
fn issue3267_bang_after_intermixed_redirect_is_word() {
    // Same once the redirection sits between the first word and `!`.
    check("coproc foo >y ! bar\n", "", FOO_NOT_FOUND, 0);
}

#[test]
fn issue3267_double_leading_redirect() {
    check("coproc >x >y foooo\n", "", NOT_FOUND, 0);
}

#[test]
fn issue3267_redirect_first_fd_var() {
    check("coproc {myfd}>x foooo\n", "", NOT_FOUND, 0);
}

// -----------------------------------------------------------------
// Guards: these stay syntax errors (rc 2), as on the base
// -----------------------------------------------------------------

#[test]
fn issue3267_dangling_redirect_still_errors() {
    check_syntax_error_near("coproc >; echo after\n", ";");
}

#[test]
fn issue3267_dangling_second_redirect_still_errors() {
    check_syntax_error_near("coproc >x >; echo after\n", ";");
}

#[test]
fn issue3267_redirect_before_brace_group_still_errors() {
    check_syntax_error_near("coproc >x { echo inner; }\n", "{");
}

#[test]
fn issue3267_redirect_before_subshell_still_errors() {
    check_syntax_error_near("coproc >x (echo inner)\n", "(");
}

#[test]
fn issue3267_redirect_before_if_still_errors() {
    check_syntax_error_near("coproc >x if true; then echo y; fi\n", "if");
}
