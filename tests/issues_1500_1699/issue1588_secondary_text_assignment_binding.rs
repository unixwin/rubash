//! Issue rubash#1588: a `${name:=word}` / `${name=word}` nested in an
//! EAGER secondary text of another expansion — the leading name's
//! subscript, a substring's offset/length texts, a patsub's pattern
//! or replacement, a case-modification pattern, or `$((...))`
//! arithmetic text — produced the right VALUE but the binding was
//! silently dropped, because those texts expand through &self paths.
//! GNU parameter_brace_expand (subst.c:9777) evaluates the operand
//! fully, so the nested assignment binds. The mirror discipline of
//! rubash#1564 still holds: an operator's WORD is not eager and must
//! not be pre-applied.
//!
//! Expected outputs verified against GNU bash 5.3.20 (script files).

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
fn issue1588_assignment_in_indexed_subscript_binds() {
    let (stdout, _, code) = rubash("a=(p q)\nx=${a[${i:=1}]}\necho \"i=$i x=$x\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "i=1 x=q\n");
}

#[test]
fn issue1588_assignment_in_assoc_subscript_binds() {
    let (stdout, _, code) = rubash("declare -A m=([k]=v)\nx=${m[${j:=k}]}\necho \"j=$j x=$x\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "j=k x=v\n");
}

#[test]
fn issue1588_plain_equals_in_subscript_binds() {
    let (stdout, _, code) = rubash("a=(p q)\nx=${a[${i=0}]}\necho \"i=$i x=$x\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "i=0 x=p\n");
}

#[test]
fn issue1588_assignment_in_substring_offset_binds() {
    let (stdout, _, code) = rubash("v=hello\nx=${v:${j:=1}}\necho \"j=$j x=$x\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "j=1 x=ello\n");
}

#[test]
fn issue1588_assignment_in_substring_length_binds() {
    let (stdout, _, code) = rubash("v=hello\nx=${v:0:${j:=2}}\necho \"j=$j x=$x\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "j=2 x=he\n");
}

#[test]
fn issue1588_assignment_in_patsub_pattern_binds() {
    let (stdout, _, code) = rubash("v=hello\nx=${v/${j:=l}/L}\necho \"j=$j x=$x\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "j=l x=heLlo\n");
}

#[test]
fn issue1588_assignment_in_patsub_replacement_binds() {
    let (stdout, _, code) = rubash("v=hello\nx=${v/l/${j:=L}}\necho \"j=$j x=$x\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "j=L x=heLlo\n");
}

#[test]
fn issue1588_assignment_in_casemod_pattern_binds() {
    let (stdout, _, code) = rubash("v=hello\nx=${v^^${j:=l}}\necho \"j=$j x=$x\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "j=l x=heLLo\n");
}

#[test]
fn issue1588_assignment_in_arithmetic_text_binds() {
    let (stdout, _, code) = rubash("x=$(( ${j:=3} + 1 ))\necho \"j=$j x=$x\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "j=3 x=4\n");
}

#[test]
fn issue1588_assignment_in_trim_pattern_binds() {
    let (stdout, _, code) = rubash("v=hello\nx=${v#${j:=h}}\necho \"j=$j x=$x\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "j=h x=ello\n");
}

#[test]
fn issue1588_operator_word_stays_non_eager() {
    // rubash#1564 discipline: when the operator does not select its
    // word, a nested assignment inside the word must NOT bind.
    let (stdout, _, code) = rubash("y=set\nx=${y:-${j:=9}}\necho \"j=[${j-unset}] x=$x\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "j=[unset] x=set\n");
}

#[test]
fn issue1588_selected_operator_word_still_binds() {
    // The selected-word path (the #1564 control) is untouched: when
    // the word IS used, its nested assignment binds.
    let (stdout, _, code) = rubash("x=${y:-${j:=9}}\necho \"j=$j x=$x\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "j=9 x=9\n");
}
