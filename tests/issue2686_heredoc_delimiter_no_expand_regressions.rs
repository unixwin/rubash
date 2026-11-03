//! rubash#2686 (deep-audit A1-F16): the word in heredoc-delimiter
//! position was run through redirect-target word expansion, EXECUTING
//! command substitutions and backquotes it contained. GNU never
//! expands the delimiter word: make_cmd.c make_here_document stores
//! `here_doc_eof = string_quote_removal(word)` and the executor
//! consumes that dequoted string; the delimiter is not a
//! redirection_expand operand (redir.c:298 covers file operands).
//! A hereSTRING word, by contrast, IS expanded (and its substitutions
//! execute) in GNU too — the last test pins that boundary.
use std::{env, path::PathBuf, process::Command};

fn marker(tag: &str) -> PathBuf {
    let path = env::temp_dir().join(format!(
        "rubash-2686-{}-{}",
        std::process::id(),
        tag
    ));
    let _ = std::fs::remove_file(&path);
    path
}

fn run(source: &str) -> (String, i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(source)
        .output()
        .expect("run rubash -c");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

#[test]
fn issue2686_delimiter_command_substitution_never_executes() {
    let path = marker("comsub");
    let source = format!(
        "cat <<$(touch {})\nx\n$(touch {})",
        path.display(),
        path.display()
    );
    let (stdout, code) = run(&source);
    assert_eq!(code, 0);
    assert_eq!(stdout, "x\n");
    assert!(
        !path.exists(),
        "delimiter command substitution executed (created {})",
        path.display()
    );
}

#[test]
fn issue2686_delimiter_backquote_never_executes() {
    let path = marker("backquote");
    let source = format!("cat <<`touch {}`\nx\n`touch {}`", path.display(), path.display());
    let (stdout, code) = run(&source);
    assert_eq!(code, 0);
    assert_eq!(stdout, "x\n");
    assert!(!path.exists(), "delimiter backquote executed");
}

#[test]
fn issue2686_delimiter_substitution_never_executes_for_builtins_and_fds() {
    let path = marker("builtin");
    let source = format!(
        ": <<$(touch {})\nx\n$(touch {})",
        path.display(),
        path.display()
    );
    let (_, code) = run(&source);
    assert_eq!(code, 0);
    assert!(!path.exists(), "builtin heredoc delimiter executed");

    let path = marker("fd");
    let source = format!(
        "cat 3<<$(touch {}) 0</dev/null\nx\n$(touch {})",
        path.display(),
        path.display()
    );
    let (_, code) = run(&source);
    assert_eq!(code, 0);
    assert!(!path.exists(), "fd heredoc delimiter executed");
}

#[test]
fn issue2686_quoted_delimiter_stays_literal_and_body_still_expands() {
    let (stdout, code) = run("x=world\ncat <<'EOF'\nhello $x\nEOF");
    assert_eq!(code, 0);
    assert_eq!(stdout, "hello $x\n");

    let (stdout, code) = run("x=world\ncat <<EOF\nhello $x\nEOF");
    assert_eq!(code, 0);
    assert_eq!(stdout, "hello world\n");
}

#[test]
fn issue2686_herestring_word_still_expands_and_executes() {
    // GNU expands a herestring word like any other word — the
    // substitution MUST run (this is the boundary of the fix: only
    // the heredoc delimiter is exempt).
    let path = marker("herestring");
    let source = format!("cat <<<$(touch {}; echo sent)", path.display());
    let (stdout, code) = run(&source);
    assert_eq!(code, 0);
    assert_eq!(stdout, "sent\n");
    assert!(
        path.exists(),
        "herestring command substitution did not execute"
    );
    let _ = std::fs::remove_file(&path);
}
