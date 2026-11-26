//! rubash#2708 — a quote character or backslash inside the tilde
//! prefix vetoes tilde expansion (GNU general.c:1210
//! bash_tilde_find_word: the scan returns *lenp == 0 on the first
//! `\`, `'` or `"` before the first `/`). Rubash's whole-word check
//! only recognized a fully-quoted word, so `~''`, `~'root'`, `~+''/x`
//! and friends expanded anyway. The veto is computed by the lexer,
//! where the raw word is still available, and carried in-band as the
//! QUOTED_WORD_PREFIX marker.
//!
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
fn quoted_empty_prefix_vetoes_tilde() {
    let (stdout, stderr, code) = rubash(
        "export HOME=/home/fake\n\
         printf '<%s>\\n' ~''\n\
         printf '<%s>\\n' ~\"\"\n\
         printf '<%s>\\n' ''~\n\
         printf '<%s>\\n' \"\"~\n\
         printf '<%s>\\n' ~+''\n\
         printf '<%s>\\n' ~''/x",
    );
    assert_eq!(stdout, "<~>\n<~>\n<~>\n<~>\n<~+>\n<~/x>\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn quote_inside_prefix_vetoes_tilde() {
    let (stdout, stderr, code) = rubash(
        "export HOME=/home/fake\n\
         printf '<%s>\\n' ~root''\n\
         printf '<%s>\\n' ~'root'\n\
         printf '<%s>\\n' ~\"root\"\n\
         printf '<%s>\\n' ~r''oot\n\
         printf '<%s>\\n' ~a\"b\"/y\n\
         printf '<%s>\\n' ~a\\ b/y",
    );
    assert_eq!(
        stdout,
        "<~root>\n<~root>\n<~root>\n<~root>\n<~ab/y>\n<~a b/y>\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn vetoed_word_still_expands_the_rest() {
    // The veto only kills the tilde pass: quote removal and parameter
    // expansion proceed as for any ordinary word (subst.c:11497,
    // t_index == 0 -> add_character, walk continues).
    let (stdout, stderr, code) = rubash(
        "export HOME=/home/fake\n\
         a='X Y'\n\
         printf '<%s>\\n' ~'x'$a",
    );
    assert_eq!(stdout, "<~xX>\n<Y>\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn quote_after_first_slash_does_not_veto() {
    let (stdout, stderr, code) = rubash(
        "export HOME=/home/fake\n\
         printf '<%s>\\n' ~/a\"b\"/y\n\
         printf '<%s>\\n' ~\n\
         printf '<%s>\\n' ~root",
    );
    assert_eq!(stdout, "</home/fake/ab/y>\n</home/fake>\n</root>\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
