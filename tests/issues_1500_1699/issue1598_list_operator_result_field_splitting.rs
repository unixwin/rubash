//! Issue rubash#1598: the result of a `-` / `:-` operator on a
//! list-valued operand (`a[@]`, `a[*]`) in an UNQUOTED whole-word
//! position kept its element boundaries instead of undergoing GNU's
//! post-expansion field splitting (subst.c expand_word_internal):
//! each `[@]` element re-splits on IFS (empty results vanish) and
//! the `[*]` joined string splits as one string — exactly the
//! treatment the plain `${a[@]}` path already applied. The quoted
//! form keeps one word per element.
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
fn issue1598_at_minus_result_resplits_elements() {
    let (stdout, _, code) = rubash("a=(x y 'z w')\nset -- ${a[@]:-dv}\necho \"n=$# j=[$*]\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=4 j=[x y z w]\n");
}

#[test]
fn issue1598_star_minus_result_resplits_joined_string() {
    let (stdout, _, code) = rubash("a=(x y 'z w')\nset -- ${a[*]:-dv}\necho \"n=$# j=[$*]\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=4 j=[x y z w]\n");
}

#[test]
fn issue1598_at_minus_splits_spaced_elements() {
    let (stdout, _, code) = rubash("b=('p q' r)\nset -- ${b[@]:-dv}\necho \"n=$# j=[$*]\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=3 j=[p q r]\n");
}

#[test]
fn issue1598_star_minus_splits_spaced_elements() {
    let (stdout, _, code) = rubash("b=('p q' r)\nset -- ${b[*]:-dv}\necho \"n=$# j=[$*]\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=3 j=[p q r]\n");
}

#[test]
fn issue1598_quoted_at_minus_keeps_element_boundaries() {
    let (stdout, _, code) =
        rubash("a=(x y 'z w')\nset -- \"${a[@]:-dv}\"\necho \"n=$# 2=$2 3=$3\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=3 2=y 3=z w\n");
}

#[test]
fn issue1598_empty_element_result_vanishes_unquoted() {
    // a=("") is set for `-` (no colon), so the value — one empty
    // element — is used; unquoted field splitting removes it.
    let (stdout, _, code) = rubash("a=('')\nset -- ${a[@]-dv}\necho \"n=$#\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=0\n");
}

#[test]
fn issue1598_for_list_resplits_elements() {
    let (stdout, _, code) =
        rubash("a=(x y 'z w')\nn=0\nfor w in ${a[@]:-dv}; do n=$((n+1)); done\necho \"n=$n\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=4\n");
}

#[test]
fn issue1598_custom_ifs_star_result_splits_on_ifs_only() {
    // With IFS=':', the joined string "p q:r" splits on ':' alone;
    // the space inside the first element survives.
    let (stdout, _, code) = rubash("IFS=:\nb=('p q' r)\nset -- ${b[*]:-dv}\necho \"n=$# j=[$*]\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=2 j=[p q:r]\n");
}

#[test]
fn issue1598_positional_at_minus_result_resplits() {
    let (stdout, _, code) = rubash("set -- 'a b' c\nset -- ${@:-dv}\necho \"n=$# j=[$*]\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=3 j=[a b c]\n");
}
