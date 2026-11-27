//! rubash#1112 — Empty array `${a@Q}` expands to a phantom `''` word.
//!
//! GNU subst.c parameter_brace_expand (@Q transformation): an empty
//! array produces ZERO words under @Q; an array holding one empty
//! element produces exactly one `''` word. The transform must not
//! manufacture a word the array does not contain.
//!
//! All expectations byte-verified against GNU bash 5.3.20 (`-c` strings).

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
fn empty_array_q_expands_to_no_words() {
    let (stdout, _, code) = rubash("a=(); echo \"[${a@Q}] n=${#a[@]}\"");
    assert_eq!(stdout, "[] n=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn one_empty_element_q_expands_to_one_quoted_word() {
    let (stdout, _, code) = rubash("a=(''); echo \"[${a@Q}] n=${#a[@]}\"");
    assert_eq!(stdout, "[''] n=1\n");
    assert_eq!(code, Some(0));
}
