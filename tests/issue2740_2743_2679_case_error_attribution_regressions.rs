//! rubash#2740 / rubash#2743 / rubash#2679 — `case` diagnostic
//! attribution.
//!
//! GNU (parse.y case_command / read_token_word with the case states):
//! the subject scan ends at the first token that is not `in`; a
//! pattern list ends at `)`; the first token that cannot continue the
//! construct is the yyerror token, reported with the ordinary
//! `syntax error near unexpected token' form and the offending line
//! echoed. rubash's case parser validated at collection boundaries
//! instead — `case in esac` blamed the construct, clause resync
//! blamed the resync token, and several faces dropped the diagnostic
//! or misattributed it to a neighbour.
//!
//! #2740: `case in esac` (missing subject AND `in`).
//! #2743: clause resync faces — junk after a pattern word, an
//!   unclosed `(`, a bare `&` after `;;`, a second `|`-pattern word.
//! #2679: the diagnostics-family faces from the first audit round —
//!   missing `)` after the subject/pattern and `case in in)`.
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
        std::env::temp_dir().join(format!("rubash-case-{tag}-{}", std::process::id()));
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
fn issue2740_case_in_esac_file() {
    let run = run_file("r2", "echo x\ncase in esac\n");
    assert_run(
        "r2",
        &run,
        "x\n",
        "case.sh: line 2: syntax error near unexpected token `esac'\n\
         case.sh: line 2: `case in esac'\n",
        2,
    );
}

#[test]
fn issue2740_case_in_esac_dash_c() {
    let run = run_c("case in esac");
    assert_run(
        "r2c",
        &run,
        "",
        "bash: -c: line 1: syntax error near unexpected token `esac'\n\
         bash: -c: line 1: `case in esac'\n",
        2,
    );
}

#[test]
fn issue2743_word_after_pattern_word() {
    let run = run_c("case x in a echo;; esac");
    assert_run(
        "r5a",
        &run,
        "",
        "bash: -c: line 1: syntax error near unexpected token `echo'\n\
         bash: -c: line 1: `case x in a echo;; esac'\n",
        2,
    );
}

#[test]
fn issue2743_word_after_open_paren_pattern() {
    let run = run_c("case x in (a echo");
    assert_run(
        "r5b",
        &run,
        "",
        "bash: -c: line 1: syntax error near unexpected token `echo'\n\
         bash: -c: line 1: `case x in (a echo'\n",
        2,
    );
}

#[test]
fn issue2743_bare_ampersand_after_clause_terminator() {
    let run = run_c("case x in *) echo;; & esac");
    assert_run(
        "r5c",
        &run,
        "",
        "bash: -c: line 1: syntax error near unexpected token `&'\n\
         bash: -c: line 1: `case x in *) echo;; & esac'\n",
        2,
    );
}

#[test]
fn issue2743_second_pattern_word_after_bar() {
    let run = run_c("case x in a|) echo;; esac");
    assert_run(
        "r5d",
        &run,
        "",
        "bash: -c: line 1: syntax error near unexpected token `)'\n\
         bash: -c: line 1: `case x in a|) echo;; esac'\n",
        2,
    );
}

#[test]
fn issue2743_word_after_pattern_word_multiline_file() {
    let run = run_file("r5e", "case x in\na echo;;\nesac\n");
    assert_run(
        "r5e",
        &run,
        "",
        "case.sh: line 2: syntax error near unexpected token `echo'\n\
         case.sh: line 2: `a echo;;'\n",
        2,
    );
}

#[test]
fn issue2679_word_after_subject_pattern_position() {
    let run = run_file("c23", "case abc in abc echo x;; esac; echo rc=$?\n");
    assert_run(
        "c23",
        &run,
        "",
        "case.sh: line 1: syntax error near unexpected token `echo'\n\
         case.sh: line 1: `case abc in abc echo x;; esac; echo rc=$?'\n",
        2,
    );
}

#[test]
fn issue2679_word_where_in_expected() {
    let run = run_file("c24", "case abc abc) echo x;; esac; echo rc=$?\n");
    assert_run(
        "c24",
        &run,
        "",
        "case.sh: line 1: syntax error near unexpected token `abc'\n\
         case.sh: line 1: `case abc abc) echo x;; esac; echo rc=$?'\n",
        2,
    );
}

#[test]
fn issue2679_case_in_in_paren() {
    let run = run_file("c25", "case in in) echo x;; esac; echo rc=$?\n");
    assert_run(
        "c25",
        &run,
        "",
        "case.sh: line 1: syntax error near unexpected token `)'\n\
         case.sh: line 1: `case in in) echo x;; esac; echo rc=$?'\n",
        2,
    );
}

#[test]
fn issue2740_wellformed_case_controls() {
    let run = run_file("ctrl", "case abc in abc) echo hit;; *) echo miss;; esac\n");
    assert_run("ctrl", &run, "hit\n", "", 0);
    // A keyword is a legal subject and a legal pattern.
    let run = run_c("case do in do) echo m;; esac");
    assert_run("kw-ctrl", &run, "m\n", "", 0);
}
