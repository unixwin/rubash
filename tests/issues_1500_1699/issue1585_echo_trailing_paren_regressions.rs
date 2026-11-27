//! Issue unixwin/rubash#1585 (arch-audit D, candidate D1): the echo
//! builtin carried a hardcoded literal special case that rewrote a
//! sole `hi)` argument to `hi`. It had been added to compensate for a
//! heredoc-body synthesis defect in the comsub-eof5 shape; that defect
//! is gone (the substitution itself yields `hi` on every route), and
//! the rewrite had become a divergence of its own — GNU echo prints
//! its arguments verbatim (builtins/echo.def).
//!
//! Verified against GNU Bash 5.3.20 (script-file probes).

use std::process::{Command, Stdio};

fn run_rubash_file(script: &str, tag: &str) -> (String, String, Option<i32>) {
    let path = std::env::temp_dir().join(format!("issue1585_{}_{}.sh", tag, std::process::id()));
    std::fs::write(&path, script).expect("write probe script");
    let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

#[test]
fn issue1585_quoted_trailing_paren_prints_verbatim() {
    let (stdout, stderr, code) = run_rubash_file("echo 'hi)'\n", "quoted");
    assert_eq!(stdout, "hi)\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1585_multi_arg_parens_print_verbatim() {
    let (stdout, _stderr, code) = run_rubash_file("echo 'a)b' 'x)' 'end)'\n", "multi");
    assert_eq!(stdout, "a)b x) end)\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1585_unquoted_paren_is_a_syntax_error_like_gnu() {
    // Control: an UNQUOTED `)` is a metacharacter — GNU rejects the
    // command outright (rc=2, two diagnostic lines incl. the source
    // echo). The special case never owned this shape either; pinning
    // it keeps the verbatim-argument tests honest about which form
    // the issue is about.
    let (stdout, stderr, code) = run_rubash_file("echo hi)\n", "unquoted");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("syntax error near unexpected token `)'"),
        "{stderr:?}"
    );
    assert!(stderr.contains("`echo hi)'"), "{stderr:?}");
    assert_eq!(code, Some(2));
}

#[test]
fn issue1585_comsub_eof5_quoted_form_still_yields_hi() {
    // The shape the removed special case compensated for: the
    // substitution's own output is `hi` (the heredoc collector, not
    // echo, owns the trailing paren), and the EOF heredoc warning
    // still fires exactly as GNU's does.
    let (stdout, stderr, code) = run_rubash_file("echo \"$(\ncat <<\\)\nhi\n))\"\n", "eof5");
    assert_eq!(stdout, "hi\n");
    assert!(
        stderr.contains("warning: here-document at line 2 delimited by end-of-file (wanted `)')"),
        "missing heredoc EOF warning: {stderr:?}"
    );
    assert_eq!(code, Some(0));
}
