//! rubash#1205 regressions: a `read` (no `-r`) whose line ends in a
//! backslash-newline continuation that reaches EOF returned status 0 with
//! the partial value. GNU read.def:802-830 swallows the `\<newline>`
//! pair and keeps reading; EOF inside the continuation is still an
//! unterminated record (eof=1), so the builtin assigns the partial value
//! and fails with status 1.
//!
//! Expectations verified byte-for-byte against WSL GNU Bash 5.3.20.

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1205-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn read_backslash_continuation_into_eof_fails() {
    let (stdout, _, code) = rubash_file("printf 'a\\\\\\n' | { read x; echo \"rc=$? x=[$x]\"; }\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1 x=[a]\n");
}

#[test]
fn read_backslash_continuation_midline_succeeds() {
    let (stdout, _, code) =
        rubash_file("printf 'a\\\\\\nb\\n' | { read y; echo \"rc=$? y=[$y]\"; }\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0 y=[ab]\n");
}

#[test]
fn read_partial_tail_at_eof_still_fails() {
    let (stdout, _, code) =
        rubash_file("printf 'a\\\\\\nb' | { read z; echo \"rc=$? z=[$z]\"; }\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1 z=[ab]\n");
}

#[test]
fn read_raw_keeps_success_at_eof() {
    // `-r` does no continuation: the backslash is literal data and a
    // newline-terminated record reads clean.
    let (stdout, _, code) =
        rubash_file("printf 'a\\\\\\n' | { read -r x; echo \"rc=$? x=[$x]\"; }\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0 x=[a\\]\n");
}
