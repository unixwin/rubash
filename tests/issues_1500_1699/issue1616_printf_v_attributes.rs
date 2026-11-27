//! Issue rubash#1616: `printf -v` bypassed variable attributes — an
//! integer target stored the formatted text unevaluated and case
//! attributes were not applied, for scalar, element and nameref targets.
//!
//! GNU printf.def -v binds through builtin_bind_variable
//! (builtins/common.c:949) -> bind_variable -> make_variable_value
//! (variables.c:2905): the stored text runs through the target's
//! att_integer evalexp or its case conversion, exactly like an ordinary
//! assignment.
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
fn issue1616_printf_v_integer_scalar_evaluates() {
    let (stdout, _, code) = rubash(r#"declare -i n; printf -v n '%s' '3+4'; echo "n=$n""#);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=7\n");
}

#[test]
fn issue1616_printf_v_integer_self_reference() {
    let (stdout, _, _) =
        rubash(r#"declare -i cnt=10; printf -v cnt '%s' 'cnt+5'; echo "cnt=$cnt""#);
    assert_eq!(stdout, "cnt=15\n");
}

#[test]
fn issue1616_printf_v_case_scalar_applies() {
    let (stdout, _, _) = rubash(r#"declare -l s; printf -v s '%s' 'ABC'; echo "s=$s""#);
    assert_eq!(stdout, "s=abc\n");
    let (stdout, _, _) = rubash(r#"declare -u u; printf -v u '%s' 'abc'; echo "u=$u""#);
    assert_eq!(stdout, "u=ABC\n");
    let (stdout, _, _) = rubash(r#"declare -c c; printf -v c '%s' 'hello world'; echo "c=$c""#);
    assert_eq!(stdout, "c=Hello world\n");
}

#[test]
fn issue1616_printf_v_indexed_element_attributes() {
    let (stdout, _, _) = rubash(r#"declare -ia a; printf -v 'a[0]' '%s' '3*2'; echo "a0=${a[0]}""#);
    assert_eq!(stdout, "a0=6\n");
    let (stdout, _, _) = rubash(r#"declare -ua a; printf -v 'a[1]' '%s' 'abc'; echo "a1=${a[1]}""#);
    assert_eq!(stdout, "a1=ABC\n");
}

#[test]
fn issue1616_printf_v_assoc_element_attributes() {
    let (stdout, _, _) = rubash(r#"declare -uA A; printf -v 'A[k]' '%s' 'abc'; echo "Ak=${A[k]}""#);
    assert_eq!(stdout, "Ak=ABC\n");
    let (stdout, _, _) = rubash(r#"declare -iA A; printf -v 'A[k]' '%s' '2*3'; echo "Ak=${A[k]}""#);
    assert_eq!(stdout, "Ak=6\n");
}

#[test]
fn issue1616_printf_v_nameref_target_attributes() {
    let (stdout, _, _) =
        rubash(r#"declare -u tgt; declare -n r=tgt; printf -v r '%s' 'abc'; echo "tgt=$tgt""#);
    assert_eq!(stdout, "tgt=ABC\n");
}
