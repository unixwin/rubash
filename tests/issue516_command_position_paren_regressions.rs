//! Issue #516: a `(` that cannot open a compound — after a reserved word's
//! grammar slot (`for()`), after word text (`echo \${x%%[<{(]*}`), after an
//! assignment prefix at EOF (`x=1 (a`) — is GNU's immediate
//! `syntax error near unexpected token `('` + source-line echo with exit 2,
//! never a compound open that swallows the rest of the input to EOF
//! (`unexpected end of file from `(' / `from `for' command`).
//!
//! GNU spec (XCU 2.3: `(` is a control operator; parse.y function_def
//! `WORD '(' ')'` shifts the paren only after exactly one ordinary word):
//! WSL GNU Bash 5.3.0 measured oracle —
//! - `for()` / `for (` -> `syntax error near unexpected token `('` at the
//!   paren + `` `for()' `` echo, rc 2, later lines never run;
//! - F181 (extglob_command_sub_diff_check.sh:31) -> near token `(' + the
//!   verbatim source line, rc 2;
//! - `x=1 (a` -> near token `(' (the assignment prefix committed
//!   simple_command, parse.y:1054);
//! - `echo (` -> the virtual `newline' token (p14), `echo (hi` -> `hi`
//!   (p21), `echo x (y` -> `(' (two words, no `(' production);
//! - complete inputs keep working: `(echo ok)`, `f() { ...; }; f`.
//!
//! Rust semantic owner: parser/for_command.rs + parse_loop.rs (the for
//! dispatch's single-paren arm and the prefix-aware push_unclosed_paren_error),
//! parser/token_actions.rs (the function-head shift's EOF naming),
//! lexer/skip.rs main_level_paren_token_name (the driver's EOF-shape scan
//! names the `(` itself for the 2+-word/redirect/assignment classes),
//! script_driver.rs (the EOF arm consults MainLevelTokenError).

#![cfg(windows)]

use std::io::Write;
use std::process::{Command, Stdio};

struct RunOutcome {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_rubash_script(script: &str) -> RunOutcome {
    let dir = std::env::temp_dir().join(format!("rubash-issue516-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue516 scratch dir");
    let script_path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&script_path).expect("write probe script");
    file.write_all(script.as_bytes())
        .expect("write probe script");
    drop(file);
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash script");
    let code = output.status.code();
    let _ = std::fs::remove_dir_all(&dir);
    RunOutcome {
        code,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn script_prefix(stderr: &str) -> String {
    stderr
        .split(": line ")
        .next()
        .map(|head| format!("{head}: "))
        .unwrap_or_default()
}

/// Corpus F126: `for()` reports the near token at the paren with the
/// source-line echo, rc 2, and the rest of the script never runs.
#[test]
fn issue516_for_empty_paren_is_near_token_error() {
    let out = run_rubash_script("for()\necho after\n");
    assert_eq!(out.code, Some(2));
    assert_eq!(out.stdout, "");
    let prefix = script_prefix(&out.stderr);
    assert_eq!(
        out.stderr,
        format!(
            "{prefix}line 1: syntax error near unexpected token `('\n{prefix}line 1: `for()'\n"
        )
    );
}

/// `for (` (spelled with the space) is the same grammar failure.
#[test]
fn issue516_for_spaced_paren_is_near_token_error() {
    let out = run_rubash_script("for (\necho after\n");
    assert_eq!(out.code, Some(2));
    assert_eq!(out.stdout, "");
    let prefix = script_prefix(&out.stderr);
    assert_eq!(
        out.stderr,
        format!(
            "{prefix}line 1: syntax error near unexpected token `('\n{prefix}line 1: `for ('\n"
        )
    );
}

/// Corpus F181: the `(` after the pattern word is a yacc token error
/// naming the `(` itself, with the verbatim source echo, rc 2.
#[test]
fn issue516_pattern_word_paren_is_near_token_error() {
    let out = run_rubash_script("x='abc{def'; echo \\${x%%[<{(]*}\necho after\n");
    assert_eq!(out.code, Some(2));
    assert_eq!(out.stdout, "");
    let prefix = script_prefix(&out.stderr);
    assert_eq!(
        out.stderr,
        format!(
            "{prefix}line 1: syntax error near unexpected token `('\n{prefix}line 1: `x='abc{{def'; echo \\${{x%%[<{{(]*}}'\n"
        )
    );
}

/// An unclosed paren group after an assignment prefix names the `(` too
/// (GNU `x=1 (a` -> near token `('`, rc 2) — never the EOF report.
#[test]
fn issue516_assignment_prefix_unclosed_paren_names_paren() {
    let out = run_rubash_script("x=1 (a\necho after\n");
    assert_eq!(out.code, Some(2));
    assert_eq!(out.stdout, "");
    let prefix = script_prefix(&out.stderr);
    assert_eq!(
        out.stderr,
        format!(
            "{prefix}line 1: syntax error near unexpected token `('\n{prefix}line 1: `x=1 (a'\n"
        )
    );
}

/// The function-head shift at EOF names GNU's virtual `newline` token.
#[test]
fn issue516_lone_word_paren_at_eof_names_newline() {
    let out = run_rubash_script("echo (\necho after\n");
    assert_eq!(out.code, Some(2));
    assert_eq!(out.stdout, "");
    let prefix = script_prefix(&out.stderr);
    assert_eq!(
        out.stderr,
        format!("{prefix}line 1: syntax error near unexpected token `newline'\n{prefix}line 1: `echo ('\n")
    );
}

/// A word after the shifted paren is named itself (GNU p21: `hi`).
#[test]
fn issue516_lone_word_paren_word_after_names_word() {
    let out = run_rubash_script("echo (hi\n");
    assert_eq!(out.code, Some(2));
    let prefix = script_prefix(&out.stderr);
    assert!(
        out.stderr.starts_with(&format!(
            "{prefix}line 1: syntax error near unexpected token `hi'\n"
        )),
        "got {:?}",
        out.stderr
    );
}

/// Complete inputs keep working: command-position subshells, function
/// definitions and `x=1 (cmd)` assignment-prefix compounds.
#[test]
fn issue516_valid_paren_forms_unchanged() {
    let out = run_rubash_script("(echo ok)\nf() { echo fb; }\nf\nx=2 (echo sub=$x)\n");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "ok\nfb\nsub=2\n");
    assert_eq!(out.stderr, "");
}
