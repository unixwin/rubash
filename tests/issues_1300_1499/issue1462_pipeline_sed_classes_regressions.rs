//! rubash#1462 — in a pipeline whose first stage is a builtin, later
//! stages named sed were silently run by an internal substitute
//! whose POSIX character classes ([[:space:]], [[:digit:]],
//! [[:alpha:]], [[:blank:]]) never matched. GNU resolves every stage
//! through PATH.
//!
//! Probe: clusters/x1/issues/issue-1462.md repro.
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
fn pipeline_sed_posix_classes_match() {
    let (stdout, stderr, code) = rubash(
        "printf 'a \\tb c\\n'  | sed 's/[[:space:]]\\+/_/g'\n\
         printf 'a1b2\\n'     | sed 's/[[:digit:]]/_/g'\n\
         printf 'foo bar\\n'  | sed 's/[[:alpha:]]*/X/'\n\
         printf 'a b\\n'      | sed 's/[[:blank:]]/_/'",
    );
    assert_eq!(stdout, "a_b_c\na_b_\nX bar\na_b\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
