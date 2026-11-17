//! rubash#3605 class D (run-comsub-posix) — a `case` that opens a
//! command substitution's body, or sits inside a nested `$(...)`
//! there, must not let the enclosing span close at the case
//! pattern `)`. The `$(` arms of the span/copier walkers skipped
//! the `(` without feeding it to the `case WORD in` tracker, so
//! `word_boundary` stayed false, `$(case` never armed the tracker,
//! and `e)` decremented the paren depth — the outer span then
//! closed at `esac)` (one `)` early), and `tokenize_comsub_body`
//! produced an unclosed CommandSubst token
//! (`unexpected EOF while looking for matching `)'`).
//!
//! GNU reads the body fresh with parse_comsub (parse.y:4451);
//! `(` is a reserved-word-permitting separator
//! (special_case_tokens, parse.y:3369-3386).
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
        std::env::temp_dir().join(format!("rubash3605-nest-{tag}-{}", std::process::id()));
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
fn issue3605_nested_case_in_comsub_body() {
    // `case` is the first word inside a nested `$(` at depth >= 2.
    // GNU: `w` vs pattern `e` -> no match -> empty expansion.
    let run = run_file(
        "nested-case",
        "z=$(echo $(case w in e) echo l;; esac))\necho \"eight=$z\"\n",
    );
    assert_eq!(run.stdout, "eight=\n", "stdout: {:?}", run.stdout);
    assert_eq!(run.stderr, "", "stderr: {:?}", run.stderr);
    assert_eq!(run.code, Some(0), "rc");
}

#[test]
fn issue3605_nested_case_subject_comsub() {
    // `$(w)` as the case subject inside a nested `$(` (the earlier
    // rubash#3605 nested_case_subst shape stays intact).
    let run = run_file(
        "nested-subject",
        "w() { printf 'e'; }\nz=$(echo $(case $(w) in e) echo l;; esac))\necho \"seven=$z\"\n",
    );
    assert_eq!(run.stdout, "seven=l\n", "stdout: {:?}", run.stdout);
    assert_eq!(run.stderr, "", "stderr: {:?}", run.stderr);
    assert_eq!(run.code, Some(0), "rc");
}

#[test]
fn issue3605_top_level_case_comsub_assignment() {
    // Top-level `x=$(case w in e) echo l;; esac)` — the whole-word
    // strip path: `w` vs `e` -> no match -> `x` empty.
    let run = run_file(
        "top-level",
        "x=$(case w in e) echo l;; esac)\necho \"nine=$x\"\n",
    );
    assert_eq!(run.stdout, "nine=\n", "stdout: {:?}", run.stdout);
    assert_eq!(run.stderr, "", "stderr: {:?}", run.stderr);
    assert_eq!(run.code, Some(0), "rc");
}

#[test]
fn issue3605_nested_case_comsub_mixed_assignment() {
    // Mixed assignment RHS embedding `$(a=$(case...); echo $a)` —
    // the scan_substitution_spans mixed path.
    let run = run_file(
        "mixed",
        "w() { printf 'e'; }\nv=$(a=$(case $(w) in e) echo k;; esac); echo \"$a\")\necho \"six=$v\"\n",
    );
    assert_eq!(run.stdout, "six=k\n", "stdout: {:?}", run.stdout);
    assert_eq!(run.stderr, "", "stderr: {:?}", run.stderr);
    assert_eq!(run.code, Some(0), "rc");
}
