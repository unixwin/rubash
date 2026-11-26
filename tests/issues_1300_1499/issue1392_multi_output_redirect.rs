//! Issue rubash#1392: stdin redirect + `> f1 > f2` never creates/truncates
//! f1 — GNU creates both (empty f1).
//!
//! GNU redir.c opens ALL output redirects in order; each file is created /
//! truncated, only the LAST becomes stdout. The in-process cat heredoc path
//! only opened the last redirect.
//!
//! Fix: src/executor/external_file_builtins.rs (heredoc branch of
//! external_cat) and src/executor/external_finish.rs (write_cat_output)
//! now open all earlier `>` redirects for their side effects.
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
fn heredoc_with_two_output_redirects_creates_both() {
    let (stdout, _, code) = rubash(
        r#"rm -f /tmp/rubash_1392a.txt /tmp/rubash_1392b.txt; cat <<H > /tmp/rubash_1392a.txt > /tmp/rubash_1392b.txt
hello
H
echo "rc=$?"; ls /tmp/rubash_1392a.txt; rm -f /tmp/rubash_1392a.txt /tmp/rubash_1392b.txt"#,
    );
    assert_eq!(code, Some(0));
    assert!(stdout.contains("rc=0"));
    assert!(stdout.contains("/tmp/rubash_1392a.txt"));
}

#[test]
fn heredoc_with_two_output_redirects_truncates_first() {
    // Pre-existing f1 with junk must be truncated to empty.
    let (stdout, _, code) = rubash(
        r#"printf 'JUNK\n' > /tmp/rubash_1392c.txt; cat <<H > /tmp/rubash_1392c.txt > /tmp/rubash_1392d.txt
hello
H
echo "rc=$?"; wc -c < /tmp/rubash_1392c.txt; rm -f /tmp/rubash_1392c.txt /tmp/rubash_1392d.txt"#,
    );
    assert_eq!(code, Some(0));
    assert!(stdout.contains("rc=0"));
    // f1 truncated to 0 bytes
    assert!(stdout.contains("\n0\n") || stdout.trim_end().ends_with("\n0"));
}
