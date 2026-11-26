//! rubash#1455 — inside double quotes, command substitution results
//! used as the word of `${param op word}` were re-quoted literally
//! (quotes became data, spaces truncated the value, backslashes were
//! swallowed). GNU: the substitution result is used as-is in the
//! already-quoted context.
//!
//! Probe: clusters/x1/issues/issue-1455.md repro.
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
fn quoted_default_word_from_comsub_is_not_requoted() {
    let (stdout, stderr, code) = rubash(
        "unset u\n\
         echo \"Z[${u:-$(echo 'a b')}]\"\n\
         printf 'B:%s\\n' \"${u:-$(printf 'x y')}\"\n\
         printf 'D:%s\\n' \"${u:-$(printf 'tab\\there')}\"\n\
         printf 'N:%s\\n' \"${u:-$(printf '')}\"",
    );
    assert_eq!(stdout, "Z[a b]\nB:x y\nD:tab\there\nN:\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
