//! Issue rubash#1599: `${var@A}` rendered wrong attributes for pure
//! scalars (reported `declare -a`) and dropped attributes on element
//! renders.
//!
//! GNU string_var_assignment (subst.c:8645) renders the variable's OWN
//! cell: a scalar cell renders the scalar form whatever the subscript,
//! an element render carries the full var_attribute_string flag set
//! (variables.c var_attribute_string; order a A f i n r t x c l u per
//! builtins/setattr.def:415), and a missing element on an attributed
//! variable renders `declare -<flags> name` with no `=value` body.
//!
//! Expected outputs verified against GNU bash 5.3.20.

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
fn issue1599_scalar_subscript_zero_renders_scalar_form() {
    let (stdout, _, code) = rubash(r#"v='hello world'; echo "R=[${v[0]@A}]""#);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "R=[v='hello world']\n");
}

#[test]
fn issue1599_empty_scalar_subscript_zero_renders_empty_value() {
    let (stdout, _, _) = rubash(r#"v=''; echo "R=[${v[0]@A}]""#);
    assert_eq!(stdout, "R=[v='']\n");
}

#[test]
fn issue1599_unset_subscript_renders_nothing() {
    let (stdout, _, _) = rubash(r#"echo "R=[${v[0]@A}]""#);
    assert_eq!(stdout, "R=[]\n");
}

#[test]
fn issue1599_sparse_missing_element_keeps_array_flag() {
    let (stdout, _, _) = rubash(r#"a=([0]=x [2]=z); echo "R=[${a[1]@A}]""#);
    assert_eq!(stdout, "R=[declare -a a]\n");
}

#[test]
fn issue1599_element_render_carries_all_attributes() {
    let (stdout, _, _) = rubash(r#"declare -ai ia=(5 6); echo "R=[${ia[1]@A}]""#);
    assert_eq!(stdout, "R=[declare -ai ia='6']\n");
}

#[test]
fn issue1599_scalar_out_of_range_subscript_renders_flags_only() {
    let (stdout, _, _) = rubash(r#"declare -i iv=42; echo "R=[${iv[1]@A}]""#);
    assert_eq!(stdout, "R=[declare -i iv]\n");
    let (stdout, _, _) = rubash(r#"declare -i iv=42; echo "R=[${iv[0]@A}]""#);
    assert_eq!(stdout, "R=[declare -i iv='42']\n");
}

#[test]
fn issue1599_assoc_element_render_carries_all_attributes() {
    let (stdout, _, _) = rubash(r#"declare -Ai im=([k]=5); echo "R=[${im[k]@A}]""#);
    assert_eq!(stdout, "R=[declare -Ai im='5']\n");
    let (stdout, _, _) = rubash(r#"declare -Ai im=([k]=5); echo "R=[${im[zz]@A}]""#);
    assert_eq!(stdout, "R=[declare -Ai im]\n");
}

#[test]
fn issue1599_scalar_render_includes_capcase_and_trace_flags() {
    let (stdout, _, _) = rubash(r#"declare -c cap=hello; echo "R=[${cap@A}]""#);
    assert_eq!(stdout, "R=[declare -c cap='Hello']\n");
    let (stdout, _, _) = rubash(r#"declare -t tr=x; echo "R=[${tr@A}]""#);
    assert_eq!(stdout, "R=[declare -t tr='x']\n");
}

#[test]
fn issue1599_negative_subscript_resolves_last_element() {
    let (stdout, _, _) = rubash(r#"a=(x y); echo "R=[${a[-1]@A}]""#);
    assert_eq!(stdout, "R=[declare -a a='y']\n");
}
