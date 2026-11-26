//! rubash#2784 — a `case` command as the first thing inside a
//! command substitution must reach the body parser, whose
//! GNU-shaped diagnostics then name the offending token. The
//! extractor's case-aware scan (the rubash#380 PST_CASEPAT port)
//! instead claims two incomplete-case shapes — `case WORD in)`
//! and `case WORD)` — by treating the substitution's closing
//! paren as case syntax, so the scan runs to end of input and the
//! driver reports `unexpected EOF while looking for matching
//! `)'` at the EOF line. GNU reads the body with parse_comsub
//! (parse.y:4451) and its grammar rejects both shapes at the
//! paren itself: `syntax error near unexpected token `)'` on the
//! body's own line.
//!
//! Expectations measured against GNU bash 5.3.20 (2026-11-09),
//! file mode, one script per shape (a body parse error aborts the
//! whole script file, so shapes cannot share one). The
//! offending-line echo is NOT pinned for the error shapes: rubash
//! echoes the stored body fragment there — the documented
//! issue2751 gap — so, exactly as the issue2785 pins do, only the
//! diagnostic line, stdout and the exit code are pinned.

use std::path::PathBuf;
use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_file(tag: &str, script: &str) -> Run {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash2784-{tag}-{}", std::process::id()));
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

fn assert_parse_diagnostic(tag: &str, run: &Run, diagnostic_line: &str) {
    assert_eq!(run.stdout, "", "{tag}: stdout");
    assert_eq!(run.code, Some(2), "{tag}: rc");
    assert!(
        run.stderr.starts_with(&format!("{diagnostic_line}\n")),
        "{tag}: stderr must start with the GNU diagnostic line\nwant first line: {diagnostic_line:?}\ngot: {:?}",
        run.stderr
    );
    let syntax_lines = run
        .stderr
        .lines()
        .filter(|line| line.contains("syntax error"))
        .count();
    assert_eq!(
        syntax_lines, 1,
        "{tag}: exactly one syntax-error line: {:?}",
        run.stderr
    );
}

#[test]
fn issue2784_case_in_immediately_closed_is_paren_diagnostic() {
    // `case x in)` — GNU's grammar has no empty pattern list
    // without `esac`; the paren is the offender, named on line 1.
    let run = run_file("in-closed", "echo $(case x in) 2>&1\necho \"rc=$?\"\n");
    assert_parse_diagnostic(
        "in-closed",
        &run,
        "case.sh: line 1: syntax error near unexpected token `)'",
    );
}

#[test]
fn issue2784_case_without_in_is_paren_diagnostic() {
    // `case x)` — the missing `in` makes the paren the offender
    // in exactly the same way.
    let run = run_file("no-in", "echo $(case x) 2>&1\necho \"rc=$?\"\n");
    assert_parse_diagnostic(
        "no-in",
        &run,
        "case.sh: line 1: syntax error near unexpected token `)'",
    );
}

#[test]
fn issue2784_esac_before_any_clause_names_esac() {
    // `case in esac` — this diagnostic already reaches the body
    // parser on the base; the pin guards that it keeps doing so.
    let run = run_file("esac", "echo $(case in esac) 2>&1\necho \"rc=$?\"\n");
    assert_parse_diagnostic(
        "esac",
        &run,
        "case.sh: line 1: syntax error near unexpected token `esac' while looking for matching `)'",
    );
}

#[test]
fn issue2784_valid_case_body_still_executes() {
    // Control: a well-formed case body is extracted whole and
    // runs; the failing command inside reports against the
    // following script line in both shells.
    let run = run_file(
        "valid",
        "echo $(case x in x) x;; esac) 2>&1\necho \"rc=$?\"\n",
    );
    assert_eq!(run.stdout, "\nrc=0\n", "valid: stdout");
    assert_eq!(
        run.stderr, "case.sh: line 2: x: command not found\n",
        "valid: stderr"
    );
    assert_eq!(run.code, Some(0), "valid: rc");
}

#[test]
fn issue2784_empty_case_still_executes() {
    // Control: `case WORD in esac` is the one empty form GNU
    // accepts (parse.y:3433-3441); it must keep extracting.
    let run = run_file("empty", "echo \"[$(case x in esac)]\"\necho \"rc=$?\"\n");
    assert_eq!(run.stdout, "[]\nrc=0\n", "empty: stdout");
    assert_eq!(run.stderr, "", "empty: stderr");
    assert_eq!(run.code, Some(0), "empty: rc");
}
