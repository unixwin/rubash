//! rubash#1087 — `compgen -W` does no quote handling on wordlist /
//! current word.
//!
//! GNU builtins/complete.def (compgen): the wordlist and the word being
//! completed go through the same quote recognition as completion itself:
//! a quoted fragment (`'"al`) still completes against `alpha`, a fully
//! quoted word (`'"alpha"'`) completes to itself, and a quote inside the
//! wordlist (`a'lpha`) is data for the match, not a grouping character
//! that hides candidates.
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
fn compgen_w_handles_quotes_in_word_and_wordlist() {
    let (stdout, stderr, code) = rubash(
        "compgen -W \"alpha beta gamma\" -- '\"al'; echo \"rc=$?\"\n\
         compgen -W \"alpha beta\" -- '\"alpha\"'; echo \"rc=$?\"\n\
         compgen -W \"a'lpha beta\" -- \"a'l\"; echo \"rc=$?\"",
    );
    assert_eq!(stdout, "alpha\nrc=0\nalpha\nrc=0\nalpha beta\nrc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
