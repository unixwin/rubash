//! rubash#1677 — for an associative array, the bare scalar append
//! `A+=scalar` REPLACED element "0" with the scalar; GNU concatenates:
//! `A[0]=${A[0]}+scalar`.
//!
//! GNU variables.c bind_variable_internal (variables.c:3138-3142): a
//! variable with an assign_func (array/assoc) under ASS_APPEND first
//! runs make_variable_value (variables.c:2905) — plain append joins the
//! current element-0 value with the new text, the integer branch adds
//! the two evalexp results — and the assign_func then stores the joined
//! value at subscript "0" (assoc) / index 0 (indexed). A missing
//! element "0" is created holding the (evaluated) appended value; other
//! keys are untouched. A plain (non-append) scalar bind still REPLACES
//! element "0" (variables.c:3139-3140).
//!
//! Expectations hand-measured against GNU bash 5.3.20 via script files.

use std::process::Command;

fn run(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue1677_assoc_scalar_append_concatenates_element_zero() {
    let (stdout, _stderr, code) = run("declare -A B=([0]=x); B+=y; echo \"B0=${B[0]}\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "B0=xy\n");
}

#[test]
fn issue1677_assoc_scalar_append_creates_missing_element_zero() {
    let (stdout, _stderr, code) = run("declare -A D; D+=z; echo \"D0=${D[0]} n=${#D[@]}\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "D0=z n=1\n");
}

#[test]
fn issue1677_assoc_scalar_append_keeps_other_keys() {
    let (stdout, _stderr, code) =
        run("declare -A K=([k]=v); K+=y; echo \"K0=${K[0]} Kk=${K[k]} n=${#K[@]}\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "K0=y Kk=v n=2\n");
}

#[test]
fn issue1677_assoc_scalar_append_accumulates_and_preserves_spaces() {
    let (stdout, _stderr, code) = run("declare -A D=([0]=x); D+=y; D+=z; echo \"D0=${D[0]}\"; \
         declare -A E=([0]=x); E+=' y'; echo \"E0=${E[0]}\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "D0=xyz\nE0=x y\n");
}

#[test]
fn issue1677_assoc_integer_scalar_append_adds() {
    let (stdout, _stderr, code) = run("declare -iA H=([0]=5); H+=3; echo \"H0=${H[0]}\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "H0=8\n");
    // A missing element "0" on an integer assoc stores the evaluated rhs.
    let (stdout, _stderr, code) = run("declare -iA H; H+='2*3'; echo \"H0=${H[0]}\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "H0=6\n");
}

#[test]
fn issue1677_non_integer_assoc_append_is_text_concat() {
    let (stdout, _stderr, code) = run("declare -A G=([0]=5); G+=3; echo \"G0=${G[0]}\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "G0=53\n");
}

#[test]
fn issue1677_declare_form_scalar_append_concatenates() {
    let (stdout, _stderr, code) = run("declare -A B=([0]=x); declare B+=y; echo \"B0=${B[0]}\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "B0=xy\n");
}

#[test]
fn issue1677_flagless_redeclare_of_integer_assoc_appends_arithmetically() {
    // The integer attribute lives on the variable, not on the flags of
    // the re-declaring command: `declare H+=3` after `declare -iA H`
    // adds (GNU variables.c make_variable_value ASS_APPEND branch), in
    // both the scalar form and the compound element-append form.
    let (stdout, _stderr, code) = run("declare -iA H=([0]=5); declare H+=3; echo \"H0=${H[0]}\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "H0=8\n");
    let (stdout, _stderr, code) =
        run("declare -iA H=([0]=5); declare H+=([0]+=3); echo \"H0=${H[0]}\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "H0=8\n");
}

#[test]
fn issue1677_plain_scalar_bind_still_replaces_element_zero() {
    // Control: the non-append scalar bind keeps replace semantics and
    // leaves the other keys alone (variables.c:3139-3140).
    let (stdout, _stderr, code) =
        run("declare -A B=([0]=x [k]=v); B=y; echo \"B0=${B[0]} Bk=${B[k]}\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "B0=y Bk=v\n");
}

#[test]
fn issue1677_explicit_subscript_append_still_concatenates() {
    // Control: `A[k]+=v` was already a concatenation before this fix.
    let (stdout, _stderr, code) = run("declare -A C=([k]=v); C[k]+=w; echo \"Ck=${C[k]}\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "Ck=vw\n");
}

#[test]
fn issue1677_compound_append_still_replaces_per_key() {
    // Control: `A+=([0]=y)` is a compound assignment — per-key replace.
    let (stdout, _stderr, code) =
        run("declare -A M=([0]=x); M+=([0]=y [k]=w); echo \"M0=${M[0]} Mk=${M[k]}\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "M0=y Mk=w\n");
}

#[test]
fn issue1677_indexed_bare_append_concatenates_element_zero() {
    // Sibling shape measured in the same pass: for an indexed array the
    // bare `a+=scalar` also binds element 0 with concatenation — it is
    // NOT an append of a new element.
    let (stdout, _stderr, code) =
        run("declare -a a=(p q); a+=z; echo \"a0=${a[0]} a1=${a[1]} n=${#a[@]}\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "a0=pz a1=q n=2\n");
}
