//! rubash#2749 — the arithmetic-`for` header is scanned by GNU's
//! reader as a matched pair (parse.y parse_matched_pair / the
//! parse_dparen header machine), and the errors belong to three
//! distinct moments that rubash's fallback used to conflate into the
//! header-validation message:
//!
//! - junk AFTER a closed header (`for ((...)) extra`): the junk is
//!   the yyerror token — GNU's reduction reaches it before header
//!   validation ever runs;
//! - a closed header with the body missing entirely: the compound
//!   EOF form (`unexpected end of file from `for' command ...`);
//! - a header that never closes (`for ((i=0;i<3;i++`): the
//!   matched-pair EOF, verbatim, at the opener's line — including
//!   through the script driver's unclosed-input route, which used to
//!   misread the pair as a command-substitution paren error.
//!
//! A malformed header with a complete body keeps the existing
//! header-validation production (semicolon-count errors). All
//! expectations measured against GNU bash 5.3.20 (2026-11-04).

use std::path::PathBuf;
use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_file(tag: &str, script: &str) -> Run {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash2749-{tag}-{}", std::process::id()));
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

#[test]
fn issue2749_junk_after_closed_header_word() {
    let run = run_c("for ((i=0;i<3;i++)) extra");
    assert_run(
        "junk-word",
        &run,
        "",
        "bash: -c: line 1: syntax error near unexpected token `extra'\n\
         bash: -c: line 1: `for ((i=0;i<3;i++)) extra'\n",
        2,
    );
}

#[test]
fn issue2749_junk_after_closed_header_number() {
    let run = run_c("for ((i=0;i<3;i++)) 42");
    assert_run(
        "junk-num",
        &run,
        "",
        "bash: -c: line 1: syntax error near unexpected token `42'\n\
         bash: -c: line 1: `for ((i=0;i<3;i++)) 42'\n",
        2,
    );
}

#[test]
fn issue2749_junk_after_closed_header_rbrace() {
    let run = run_c("for ((i=0;i<3;i++)) }");
    assert_run(
        "junk-rbrace",
        &run,
        "",
        "bash: -c: line 1: syntax error near unexpected token `}'\n\
         bash: -c: line 1: `for ((i=0;i<3;i++)) }'\n",
        2,
    );
}

#[test]
fn issue2749_junk_after_closed_header_rparen() {
    let run = run_c("for ((i=0;i<3;i++)) )");
    assert_run(
        "junk-rparen",
        &run,
        "",
        "bash: -c: line 1: syntax error near unexpected token `)'\n\
         bash: -c: line 1: `for ((i=0;i<3;i++)) )'\n",
        2,
    );
}

#[test]
fn issue2749_junk_after_closed_header_ampersand() {
    let run = run_c("for ((i=0;i<3;i++)) &");
    assert_run(
        "junk-amp",
        &run,
        "",
        "bash: -c: line 1: syntax error near unexpected token `&'\n\
         bash: -c: line 1: `for ((i=0;i<3;i++)) &'\n",
        2,
    );
}

#[test]
fn issue2749_junk_after_bad_header_still_blames_junk() {
    // Reduction order: the post-header token is reported even when
    // the header itself is malformed.
    let run = run_c("for ((i=0;i<3)) extra");
    assert_run(
        "badhdr-extra",
        &run,
        "",
        "bash: -c: line 1: syntax error near unexpected token `extra'\n\
         bash: -c: line 1: `for ((i=0;i<3)) extra'\n",
        2,
    );
}

#[test]
fn issue2749_closed_header_no_body_is_compound_eof_file() {
    let run = run_file("eof", "for ((i=0;i<3;i++))\n");
    assert_run(
        "eof",
        &run,
        "",
        "case.sh: line 2: syntax error: unexpected end of file from `for' command on line 1\n",
        2,
    );
}

#[test]
fn issue2749_unclosed_header_is_matched_pair_eof_dash_c() {
    let run = run_c("for ((i=0;i<3;i++");
    assert_run(
        "unclosed-c",
        &run,
        "",
        "bash: -c: line 1: unexpected EOF while looking for matching `)'\n",
        2,
    );
}

#[test]
fn issue2749_unclosed_header_is_matched_pair_eof_file() {
    let run = run_file("unclosed", "for ((i=0;i<3;i++");
    assert_run(
        "unclosed-file",
        &run,
        "",
        "case.sh: line 1: unexpected EOF while looking for matching `)'\n",
        2,
    );
}

#[test]
fn issue2749_bad_header_no_body_is_compound_eof() {
    let run = run_c("for ((i=0))");
    assert_run(
        "bad2",
        &run,
        "",
        "bash: -c: line 2: syntax error: unexpected end of file from `for' command on line 1\n",
        2,
    );
}

#[test]
fn issue2749_good_header_unclosed_body_is_compound_eof() {
    let run = run_c("for ((i=0;i<3;i++)); do echo x");
    assert_run(
        "doeof",
        &run,
        "",
        "bash: -c: line 2: syntax error: unexpected end of file from `for' command on line 1\n",
        2,
    );
}

#[test]
fn issue2749_wellformed_arith_for_control() {
    let run = run_c("for ((i=0;i<3;i++)); do echo $i; done");
    assert_run("ctrl", &run, "0\n1\n2\n", "", 0);
}
