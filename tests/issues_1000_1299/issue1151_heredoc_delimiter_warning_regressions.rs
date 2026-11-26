//! rubash#1151 — Quoted-heredoc-delimiter warning has stray \023
//! byte.
//!
//! GNU parse.y (read_command / heredoc collection): when the input
//! ends before the heredoc delimiter line appears, the warning is
//! `here-document at line N delimited by end-of-file (wanted \`$D')`
//! in clean text — no control bytes. A quoted delimiter (`<<"$D"`)
//! names the LITERAL `$D` (quotes removed, no expansion), so the
//! heredoc runs to end-of-file and the following lines are its body.
//!
//! Expectations verified against GNU bash 5.3.20 (`-c` string); the
//! byte-level point of the pin is the ABSENCE of \x13 in stderr.

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn quoted_heredoc_delimiter_warning_is_clean_text() {
    let (stdout, stderr, code) = rubash("D=EOF; cat <<\"$D\"\nhello\nEOF\necho rc=$?");
    // The delimiter is the literal text $D, never matched: the rest of
    // the string is heredoc body, printed by cat.
    assert_eq!(stdout, "hello\nEOF\necho rc=$?\n");
    assert!(
        stderr.contains("warning: here-document") && stderr.contains("wanted `$D'"),
        "stderr: {stderr}"
    );
    assert!(
        !stderr.contains('\u{13}'),
        "stderr must not carry the stray \\023 byte: {stderr:?}"
    );
    assert_eq!(code, Some(0));
}
