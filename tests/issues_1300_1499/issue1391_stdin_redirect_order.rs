//! Issue rubash#1391: `read`/`mapfile` use the FIRST stdin redirect when
//! `< file` precedes `<<< herestring` — GNU uses the LAST.
//!
//! GNU redir.c applies redirects in order; the last fd-0 input redirect wins.
//! The read builtin was checking `cmd.redirect_in` (the first file redirect)
//! before the herestring path, so `< f <<< "over"` read the file instead of
//! "over".
//!
//! Fix: src/executor/read_io.rs detects when the last stdin redirect is a
//! herestring and skips the file-redirect path.
//!
//! Expected outputs verified against GNU bash 5.3.

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
fn read_last_stdin_redirect_wins_file_then_herestring() {
    let (stdout, _, code) = rubash(
        r#"echo filecontent > /tmp/rubash_1391.txt; read x < /tmp/rubash_1391.txt <<< "over"; echo "x=[$x]"; rm -f /tmp/rubash_1391.txt"#,
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "x=[over]\n");
}

#[test]
fn read_last_stdin_redirect_wins_herestring_then_file() {
    let (stdout, _, code) = rubash(
        r#"echo filecontent > /tmp/rubash_1391b.txt; read y <<< "over" < /tmp/rubash_1391b.txt; echo "y=[$y]"; rm -f /tmp/rubash_1391b.txt"#,
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "y=[filecontent]\n");
}

#[test]
fn read_explicit_fd0_last_wins() {
    let (stdout, _, code) = rubash(
        r#"echo filecontent > /tmp/rubash_1391c.txt; read x 0< /tmp/rubash_1391c.txt <<< "over"; echo "x=[$x]"; rm -f /tmp/rubash_1391c.txt"#,
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "x=[over]\n");
}
