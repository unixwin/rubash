//! Issue #513: a `)` inside a `$(...)` body comment must not terminate the
//! substitution — the comment extends to end of line (GNU prints `[hi]`).
//!
//! GNU spec (XCU 2.3 Token Recognition; parse.y read_token `#` branch,
//! parse.y:3630-3643): `#` opens a comment at a token boundary and the
//! comment runs through the newline, so `$(echo hi  # c with ) paren`
//! keeps scanning for its closer on the next line and `echo "[$(...)]"`
//! prints `[hi]`. Corpus F049 (cmdsub_comment_diff_check.sh:14, label
//! paren-in-comment). rubash cut the substitution body at the comment's
//! `)` in the executor's substitution-span scanner
//! (executor/substitution_metadata.rs scan_substitution_spans — the
//! fragment fast path feeding expand_simple_substitution_fragments), in
//! the embedded-parameter walker's `$(` arm
//! (executor/embedded_parameters.rs), and in the word-level quote-removal
//! copy scanners (lexer/quotes.rs copy_dollar_paren_body_raw{,_cursor});
//! all now run the same boundary-gated comment machine as
//! lexer/skip.rs skip_cmd_subst.
//!
//! WSL GNU Bash 5.3.0 measured (2026-10-12): `[hi]` for the corpus and
//! its unquoted-prefix / assignment forms; `p#q` literal (mid-word `#`
//! is word text); `echo "$(true # )"` is an unclosed-substitution syntax
//! error in GNU too (the `)` is comment text).
//!
//! Rust semantic owner: the four scanners above.

#![cfg(windows)]

use std::io::Write;
use std::process::{Command, Stdio};

struct RunOutcome {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_rubash_script(script: &str) -> RunOutcome {
    let dir = std::env::temp_dir().join(format!("rubash-issue513-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue513 scratch dir");
    let script_path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&script_path).expect("create probe script");
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

/// Corpus F049: the comment's `)` must not close the substitution; the
/// closer on the next line does (GNU `[hi]`).
#[test]
fn issue513_paren_in_comment_extends_to_end_of_line() {
    let out = run_rubash_script("echo \"[$(echo hi  # c with ) paren\n)]\"\n");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "[hi]\n");
    assert_eq!(out.stderr, "");
}

/// The same body shape in an unquoted word with literal text around the
/// substitution (GNU `[hi]`).
#[test]
fn issue513_unquoted_prefix_form() {
    let out = run_rubash_script("echo [$(echo hi  # c with ) paren\n)]\n");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "[hi]\n");
    assert_eq!(out.stderr, "");
}

/// Assignment RHS keeps the full body (GNU stores `hi`).
#[test]
fn issue513_assignment_rhs_form() {
    let out = run_rubash_script("w=\"$(echo hi  # c with ) paren\n)\"\necho \"[$w]\"\n");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "[hi]\n");
    assert_eq!(out.stderr, "");
}

/// A mid-word `#` is word text, never a comment introducer (GNU `p#q`).
#[test]
fn issue513_midword_hash_stays_literal() {
    let out = run_rubash_script("echo \"$(echo p#q)\"\n");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "p#q\n");
    assert_eq!(out.stderr, "");
}

/// A comment that swallows the closer leaves the substitution unclosed —
/// GNU reports `unexpected EOF while looking for matching ')')` here too.
#[test]
fn issue513_comment_swallowing_closer_stays_unclosed() {
    let out = run_rubash_script("echo \"$(true # )\"\n");
    assert_eq!(out.code, Some(2));
    assert!(out
        .stderr
        .contains("unexpected EOF while looking for matching `)'"));
}

/// Case-pattern parens and the comment machine coexist: the trailing
/// comment swallows its `)`, so the real closer arrives on the next line
/// (GNU runs the `z` branch and prints `[z]`).
#[test]
fn issue513_comment_after_case_body() {
    let out = run_rubash_script("w=\"$(case z in z) echo z ;; esac # tail\n)\"\necho \"[$w]\"\n");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "[z]\n");
    assert_eq!(out.stderr, "");
}
