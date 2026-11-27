//! rubash#1353 — `[[ ]]` syntax-error token text differed for embedded
//! `<` `>` `&` `|` (rc=2 in both shells; only the wording/token shape
//! was wrong).
//!
//! GNU (parse.y cond-parser diagnostics): a leading-digit operand glued
//! to the operator reports "unexpected token `1' in conditional
//! command" + "syntax error near `1<'"; a bare `&`/`|` reports
//! "unexpected token `&', conditional binary operator expected" +
//! "syntax error near `&3'".
//! Probe: clusters/k9/probes/p1353.sh (split per form: a syntax error
//! aborts the script, so each form is its own script).
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1353-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run_form(tag: &str, line: &str) -> (String, String, Option<i32>) {
    let dir = tempdir(tag);
    let script = dir.join("probe.sh");
    std::fs::write(&script, format!("{line}; echo \"rc=$?\"\n")).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let result = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn cond_syntax_error_token_text_matches_gnu() {
    let cases: [(&str, &str, &str); 4] = [
        (
            "[[ 1<<4 -eq 16 ]]",
            "unexpected token `1' in conditional command",
            "syntax error near `1<'",
        ),
        (
            "[[ 5&3 -eq 1 ]]",
            "unexpected token `&', conditional binary operator expected",
            "syntax error near `&3'",
        ),
        (
            "[[ 1>2 -eq 0 ]]",
            "unexpected token `1' in conditional command",
            "syntax error near `1>'",
        ),
        (
            "[[ 5|3 -eq 1 ]]",
            "unexpected token `|', conditional binary operator expected",
            "syntax error near `|3'",
        ),
    ];
    for (i, (line, tok, near)) in cases.iter().enumerate() {
        let (stdout, stderr, code) = run_form(&format!("f{i}"), line);
        assert_eq!(stdout, "", "{line}");
        assert_eq!(code, Some(2), "{line}");
        assert!(stderr.contains(tok), "{line} stderr: {stderr}");
        assert!(stderr.contains(near), "{line} stderr: {stderr}");
    }
}
