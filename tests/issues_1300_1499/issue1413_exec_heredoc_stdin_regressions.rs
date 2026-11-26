//! rubash#1413 — `exec <<EOF` / `exec <<<str` did not redirect the
//! shell's stdin persistently; GNU does, so a following `cat` reads
//! the heredoc / herestring.
//!
//! Probe: clusters/k9/probes/p1413.sh.
//! Expectations verified against GNU bash 5.3.20 (script files).

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
fn exec_heredoc_and_herestring_redirect_shell_stdin() {
    let (stdout, stderr, code) = rubash("exec <<EOF2\nhello\nEOF2\ncat\nexec <<<hi; cat");
    assert_eq!(stdout, "hello\nhi\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
