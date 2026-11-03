//! rubash#2704 — tilde expansion was a whole-word shortcut: the prefix
//! resolver glued the remainder of the word verbatim, so expansions in
//! the remainder never ran (`~/$a` printed the literal `$a`, and
//! `~/"$a"` leaked the PARAM_NAME_END_MARKER carrier byte).
//!
//! GNU subst.c:11497-11512 (expand_word_internal, case '~'): the tilde
//! prefix is one pass INSIDE the word walk — the expanded prefix joins
//! via add_quoted_string (subst.c:11862, CTLESC-protected) and the walk
//! CONTINUES with the remainder of the word.
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
fn tilde_remainder_expands_and_splits() {
    let (stdout, stderr, code) = rubash(
        "export HOME=/home/fake\n\
         a='X Y'; b='3 4'\n\
         printf '<%s>\\n' ~/$a\n\
         printf '<%s>\\n' ~/${a}\n\
         printf '<%s>\\n' ~root/$a\n\
         printf '<%s>\\n' ~/$a\"$b\"\n\
         printf '<%s>\\n' ~/$((6*7))\n\
         printf '<%s>\\n' ~/$a/x",
    );
    assert_eq!(
        stdout,
        "</home/fake/X>\n<Y>\n\
         </home/fake/X>\n<Y>\n\
         </root/X>\n<Y>\n\
         </home/fake/X>\n<Y3 4>\n\
         </home/fake/42>\n\
         </home/fake/X>\n<Y/x>\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn tilde_remainder_quoted_segments_stay_protected() {
    let (stdout, stderr, code) = rubash(
        "export HOME=/home/fake\n\
         a='X Y'\n\
         printf '<%s>\\n' ~/\"$a\"\n\
         printf '<%s>\\n' ~/a\"b c\"d",
    );
    assert_eq!(stdout, "</home/fake/X Y>\n</home/fake/ab cd>\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn tilde_remainder_command_substitution_runs() {
    let (stdout, stderr, code) = rubash(
        "export HOME=/home/fake\n\
         printf '<%s>\\n' ~/$(echo 'p q')",
    );
    assert_eq!(stdout, "</home/fake/p>\n<q>\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn tilde_remainder_keeps_side_effects_and_unresolved_prefix() {
    // ${c:=Z} in the remainder both expands and assigns; an unresolved
    // prefix (`~$a` names no user) stays literal and the walk still
    // expands the remainder (GNU: tilde_expand returns the word
    // unchanged, expand_word_internal keeps walking).
    let (stdout, stderr, code) = rubash(
        "export HOME=/home/fake\n\
         a='X Y'\n\
         printf '<%s>\\n' ~$a/x\n\
         printf '<%s>\\n' ~/${c:=Z}\n\
         echo \"c=$c\"",
    );
    assert_eq!(stdout, "<~X>\n<Y/x>\n</home/fake/Z>\nc=Z\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn tilde_prefix_itself_is_quoted_data() {
    // The expanded prefix joins as a quoted string (subst.c:11862
    // add_quoted_string): a space inside $HOME must not field-split
    // even when the remainder makes the word split-eligible.
    let (stdout, stderr, code) = rubash(
        "export HOME='/h f'\n\
         a='X Y'\n\
         printf '<%s>\\n' ~/$a\n\
         printf '<%s>\\n' ~",
    );
    assert_eq!(stdout, "</h f/X>\n<Y>\n</h f>\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
