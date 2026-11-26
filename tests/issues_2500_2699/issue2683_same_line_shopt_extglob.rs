//! rubash#2683: same-line `shopt -s extglob` and the extglob gate seen
//! by a later `[[ ]]` on that line.
//!
//! GNU truth (oracle-verified; it corrects the issue text twice):
//! 1. The issue's glued repro `[[ abc == @(abc)]]` is a syntax error in
//!    GNU too (the `)]]` needs a space). The live shape is `@(abc) ]]`.
//! 2. Inside `[[ lhs == pattern ]]`, extended-glob pattern lexing does
//!    NOT depend on the extglob option at all: cond_term sets PST_EXTPAT
//!    for `=`/`==`/`!=` (parse.y:5158-5163), the RHS read forces
//!    `extended_glob = 1` under PST_EXTPAT (parse.y:5203-5206), and
//!    evaluation forces it again (execute_cmd.c:4108-4111). So
//!    `[[ abc == @(abc) ]]` matches with extglob never enabled.
//! 3. The option's real visibility boundary is the READ UNIT: a whole
//!    line is lexed before any of it executes, so a same-line
//!    `shopt -s extglob` is invisible to later commands ON THAT LINE
//!    (case patterns / glob words: syntax error), and takes effect from
//!    the next line on (shopt_set_extglob, builtins/shopt.def:638-640,
//!    sets the parser's extended_glob when the builtin RUNS — after the
//!    current line has been read).
//!
//! Expectations verified byte-for-byte against GNU Bash 5.3.20
//! script-file probes (self-compiled oracle).

use std::process::Command;

struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "rubash-issue2683-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        Scratch(dir)
    }
    fn file(&self, script: &str) -> (String, String, Option<i32>) {
        std::fs::write(self.0.join("case.sh"), script).expect("write case.sh");
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg("case.sh")
            .current_dir(&self.0)
            .output()
            .expect("run rubash file");
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
        )
    }
    fn dash_c(&self, script: &str) -> (String, String, Option<i32>) {
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg("-c")
            .arg(script)
            .current_dir(&self.0)
            .output()
            .expect("run rubash -c");
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
        )
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn check_ok(run: (String, String, Option<i32>), expected_stdout: &str, shape: &str) {
    let (stdout, stderr, code) = run;
    assert_eq!(code, Some(0), "shape {shape} failed\nstderr: {stderr}");
    assert_eq!(stderr, "", "shape {shape} wrote to stderr");
    assert_eq!(stdout, expected_stdout, "shape: {shape}");
}

fn check_syntax_error(run: (String, String, Option<i32>), shape: &str) {
    let (stdout, stderr, code) = run;
    assert_eq!(
        code,
        Some(2),
        "shape {shape}\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert_eq!(stdout, "", "shape {shape} wrote to stdout");
    assert!(
        stderr.contains("syntax error"),
        "shape {shape} stderr lacks a syntax error: {stderr}"
    );
}

/// The core cell: no shopt anywhere — `[[ == ]]` extglob patterns are
/// lexed and matched unconditionally in GNU.
#[test]
fn cond_extglob_pattern_needs_no_shopt() {
    let s = Scratch::new();
    check_ok(
        s.file("[[ abc == @(abc) ]] && echo MATCH || echo NOMATCH\n"),
        "MATCH\n",
        "bare cond extglob",
    );
}

/// The issue's shape (spaced form), file route: a same-line
/// `shopt -s extglob` between two conditionals.
#[test]
fn same_line_shopt_between_conditionals_file() {
    let s = Scratch::new();
    check_ok(
        s.file("[[ abc == @(abc) ]]; shopt -s extglob; [[ abc == @(abc) ]] && echo SECOND-MATCH || echo SECOND-NOMATCH\n"),
        "SECOND-MATCH\n",
        "same-line shopt, file",
    );
}

/// The issue's shape via `-c` (whole-string parse in both shells).
#[test]
fn same_line_shopt_between_conditionals_dash_c() {
    let s = Scratch::new();
    check_ok(
        s.dash_c("[[ abc == @(abc) ]]; shopt -s extglob; [[ abc == @(abc) ]] && echo SECOND-MATCH || echo SECOND-NOMATCH"),
        "SECOND-MATCH\n",
        "same-line shopt, -c",
    );
}

/// The three commands on separate lines: the shopt line executes
/// before the second conditional's line is read.
#[test]
fn shopt_on_own_line_before_conditional() {
    let s = Scratch::new();
    check_ok(
        s.file("[[ abc == @(abc) ]]\nshopt -s extglob\n[[ abc == @(abc) ]] && echo SECOND-MATCH || echo SECOND-NOMATCH\n"),
        "SECOND-MATCH\n",
        "shopt on own line",
    );
}

/// Control: extglob enabled on an earlier line — matching AND
/// non-matching patterns both behave.
#[test]
fn preenabled_extglob_match_and_nomatch() {
    let s = Scratch::new();
    check_ok(
        s.file("shopt -s extglob\n[[ abc == @(abc) ]] && echo MATCH; [[ xyz == @(abc) ]] || echo NOMATCH-OK\n"),
        "MATCH\nNOMATCH-OK\n",
        "pre-enabled",
    );
}

/// Chain: on/off flips across lines — the conditional result must not
/// depend on the option in EITHER state (state is fed forward line by
/// line, the assertion is on the final conditional).
#[test]
fn shopt_flip_chain_conditional_unaffected() {
    let s = Scratch::new();
    check_ok(
        s.file("shopt -s extglob\nshopt -u extglob\n[[ abc == @(abc) ]] && echo STILL-MATCH || echo BROKEN\n"),
        "STILL-MATCH\n",
        "on-then-off chain",
    );
}

/// Line-separated visibility for a real option consumer: a `case`
/// pattern on the NEXT line sees the executed shopt.
#[test]
fn case_pattern_sees_shopt_from_previous_line() {
    let s = Scratch::new();
    check_ok(
        s.file("shopt -s extglob\ncase abc in @(abc)) echo CASEHIT;; *) echo CASENO;; esac\n"),
        "CASEHIT\n",
        "case next line",
    );
}

/// Line-separated visibility for a glob word on the NEXT line: the
/// unmatched extglob pattern stays literal (empty scratch dir).
#[test]
fn glob_word_sees_shopt_from_previous_line() {
    let s = Scratch::new();
    check_ok(
        s.file("shopt -s extglob\necho @(abc)\n"),
        "@(abc)\n",
        "glob next line",
    );
}

/// Same-line invisibility: GNU lexes the whole line before executing
/// the shopt, so a case pattern later on the SAME line is a syntax
/// error even though the shopt textually precedes it.
#[test]
fn case_pattern_blind_to_same_line_shopt() {
    let s = Scratch::new();
    check_syntax_error(
        s.file("shopt -s extglob; case abc in @(abc)) echo CASEHIT;; *) echo CASENO;; esac\n"),
        "case same line",
    );
}

/// Same-line invisibility for a glob word.
#[test]
fn glob_word_blind_to_same_line_shopt() {
    let s = Scratch::new();
    check_syntax_error(s.file("shopt -s extglob; echo @(abc)\n"), "glob same line");
}
