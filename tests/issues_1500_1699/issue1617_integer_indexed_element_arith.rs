//! Issue rubash#1617: `declare -i` indexed-array element assignment
//! evaluated the value with a crippled arithmetic evaluator that only
//! understood literals and `+`, so `a[0]='2*3'` stored 0.
//!
//! GNU assign_array_element (arrayfunc.c:334) -> bind_array_variable ->
//! make_array_variable_value (arrayfunc.c:166) -> make_variable_value
//! (variables.c:2905) runs the FULL evalexp (expr.c) on the element
//! text, for `=` and for both sides of `+=`.
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
fn issue1617_indexed_element_full_arithmetic() {
    let script = r#"declare -ia a
a[0]='2*3'; a[1]='5-4'; a[2]='2**3'; a[3]='(2+1)*2'; x=5; a[4]='x'; a[5]='x+2'
echo "mul=${a[0]} sub=${a[1]} pow=${a[2]} par=${a[3]} var=${a[4]} mix=${a[5]}""#;
    let (stdout, _, code) = rubash(script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "mul=6 sub=1 pow=8 par=6 var=5 mix=7\n");
}

#[test]
fn issue1617_indexed_element_more_operators() {
    let script = r#"declare -ia a
a[0]='7/2'; a[1]='7%3'; a[2]='2<<3'; a[3]='-5'; a[4]='1>0?9:1'; a[5]='a[0]+2'
echo "div=${a[0]} mod=${a[1]} shl=${a[2]} neg=${a[3]} tern=${a[4]} elem=${a[5]}""#;
    let (stdout, _, _) = rubash(script);
    assert_eq!(stdout, "div=3 mod=1 shl=16 neg=-5 tern=9 elem=5\n");
}

#[test]
fn issue1617_indexed_element_append_arithmetic() {
    let (stdout, _, _) = rubash(r#"declare -ia a=(1); a[0]+='2*3'; echo "v=${a[0]}""#);
    assert_eq!(stdout, "v=7\n");
}

#[test]
fn issue1617_declare_init_integer_elements_evaluate() {
    let (stdout, _, _) = rubash(r#"declare -ia a=('2*3' '4+5'); echo "v=${a[0]} ${a[1]}""#);
    assert_eq!(stdout, "v=6 9\n");
}

#[test]
fn issue1617_scalar_append_to_integer_array_evaluates() {
    let (stdout, _, _) = rubash(r#"declare -i ia=(1); ia+='2*3'; echo "v=${ia[0]}""#);
    assert_eq!(stdout, "v=7\n");
}
