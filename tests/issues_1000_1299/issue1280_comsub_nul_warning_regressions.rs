//! rubash#1280 — Missing warning on NUL bytes in command
//! substitution (GNU: ignored null byte in input).
//!
//! GNU subst.c command_substitute: NUL bytes in captured output are
//! dropped (they cannot live in a shell string) AND the drop is
//! reported — `warning: command substitution: ignored null byte in
//! input`, once per substitution that contained one.
//!
//! Expectations verified against GNU bash 5.3.20 (`-c` string): the
//! surviving value and the od rendering are byte-shaped (od spacing
//! normalized), the warning text is matched on its stable core.

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
fn nul_bytes_in_comsub_warn_and_drop() {
    let (stdout, stderr, code) =
        rubash("v=$(printf 'a\\0b'); echo \"len=${#v}\"\necho $(printf 'a\\0b') | od -An -c");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "len=2");
    let od_line: String = lines[1].split_whitespace().collect::<Vec<_>>().join(" ");
    assert_eq!(od_line, "a b \\n", "od rendering of the surviving bytes");
    assert_eq!(
        stderr
            .matches("warning: command substitution: ignored null byte in input")
            .count(),
        2,
        "one warning per affected substitution; stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}
