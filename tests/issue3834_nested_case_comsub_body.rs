//! rubash#3834 — a `case` clause body inside `$(...)` whose nested
//! `$(case ... esac)` is followed by trailing text must not report a
//! syntax error. The parser-side comsub extractor
//! (`dollar_command_substitution` /
//! `update_command_substitution_reserved_word_depth`) still used the
//! `)`-then-evidence FORWARD lookahead for `esac`: after the inner
//! `esac)` it scanned ahead, found the OUTER clause's `;;`, and judged
//! the inner `esac` pattern text — `case_depth` never returned to the
//! outer level, the outer `)` was not counted, and the extracted body
//! carried a stray `)` into the real parse
//! (`syntax error near unexpected token ')'`).
//!
//! GNU resolves the same ambiguity BACKWARD: `esac' is pattern text
//! only when `last_read_token` is `|' or the pattern-list `('
//! (parse.y:3177-3186 CHECK_FOR_RESERVED_WORD, 3181/3183 Posix rules
//! 4). parse_comsub (parse.y:4451) hands the body to a fresh parser,
//! so each substitution level keeps its own case state; the witness
//! is the previous significant unquoted character — the same
//! `prev_sig` machine the skip.rs / embedded_mutations walkers
//! already carry (rubash#380).
//!
//! Expectations measured against GNU bash 5.3.20 (2026-11-15),
//! file mode, one script per shape.

use std::path::PathBuf;
use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_file(tag: &str, script: &str) -> Run {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash3834-{tag}-{}", std::process::id()));
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

#[test]
fn issue3834_case_body_nested_comsub_trailing_text() {
    // Canonical discriminator: inner `$(case ... esac)` followed by `z`
    // inside an outer case clause body. GNU: `[qokz]` rc=0.
    let run = run_file(
        "canonical",
        "x=$(case z in z) echo q$(case y in y) echo ok;; esac)z;; esac); echo \"[$x]\"\n",
    );
    assert_eq!(run.stdout, "[qokz]\n", "stdout: {:?}", run.stdout);
    assert_eq!(run.stderr, "", "stderr: {:?}", run.stderr);
    assert_eq!(run.code, Some(0), "rc");
}

#[test]
fn issue3834_case_body_nested_comsub_trailing_word() {
    // Same shape, different trailing word — `w` instead of `z`.
    // GNU: `[qokw]` rc=0.
    let run = run_file(
        "trailing-word",
        "x=$(case z in z) echo q$(case y in y) echo ok;; esac)w;; esac); echo \"[$x]\"\n",
    );
    assert_eq!(run.stdout, "[qokw]\n", "stdout: {:?}", run.stdout);
    assert_eq!(run.stderr, "", "stderr: {:?}", run.stderr);
    assert_eq!(run.code, Some(0), "rc");
}

#[test]
fn issue3834_case_body_nested_comsub_in_if_condition() {
    // Same nested shape inside an `if` condition — the comsub sits in
    // command position rather than assignment RHS. GNU: `[qokz]` rc=0.
    let run = run_file(
        "if-condition",
        "if x=$(case z in z) echo q$(case y in y) echo ok;; esac)z;; esac); then echo \"[$x]\"; fi\n",
    );
    assert_eq!(run.stdout, "[qokz]\n", "stdout: {:?}", run.stdout);
    assert_eq!(run.stderr, "", "stderr: {:?}", run.stderr);
    assert_eq!(run.code, Some(0), "rc");
}

#[test]
fn issue3834_case_pattern_esac_after_pipe_still_pattern() {
    // Control: `esac` right after `|` inside a pattern list IS pattern
    // text (parse.y:3181 Posix rule 4) — the backward witness must not
    // regress it. GNU: `matched` rc=0.
    let run = run_file(
        "pattern-esac",
        "case foo in foo|esac) echo matched;; esac\n",
    );
    assert_eq!(run.stdout, "matched\n", "stdout: {:?}", run.stdout);
    assert_eq!(run.stderr, "", "stderr: {:?}", run.stderr);
    assert_eq!(run.code, Some(0), "rc");
}

#[test]
fn issue3834_case_terminator_esac_after_terminator() {
    // Control: the real ESAC keyword after `;;` still decrements
    // `case_depth` so the enclosing `)` closes the substitution.
    // GNU: `[hi]` rc=0.
    let run = run_file(
        "terminator-esac",
        "x=$(case z in z) echo hi;; esac); echo \"[$x]\"\n",
    );
    assert_eq!(run.stdout, "[hi]\n", "stdout: {:?}", run.stdout);
    assert_eq!(run.stderr, "", "stderr: {:?}", run.stderr);
    assert_eq!(run.code, Some(0), "rc");
}
