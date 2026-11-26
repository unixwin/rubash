//! rubash#1088 — `compgen -F` clears caller's COMP_WORDS.
//!
//! GNU builtins/complete.def: a function invoked via `compgen -F` runs
//! with the completion variables set for the completion attempt; it
//! must not destroy the caller's own COMP_WORDS array in the process.
//! Here the caller's function prints COMP_WORDS from inside the -F call
//! and must still see the caller's values.
//!
//! Expectations byte-verified against GNU bash 5.3.20 (`-c` string).

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
fn compgen_f_preserves_caller_comp_words() {
    let (stdout, _, code) = rubash(
        "_g(){ printf 'WORDS=(%s)\\n' \"${COMP_WORDS[*]}\"; }\n\
         COMP_WORDS=(mycmd hello); COMP_CWORD=1; COMP_LINE=\"mycmd hello\"; COMP_POINT=11\n\
         compgen -F _g -- hello 2>/dev/null; echo \"rc=$?\"",
    );
    assert_eq!(stdout, "WORDS=(mycmd hello)\nrc=1\n");
    assert_eq!(code, Some(0));
}
