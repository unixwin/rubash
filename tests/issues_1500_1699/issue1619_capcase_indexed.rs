//! Issue rubash#1619: `declare -c` (capcase) was dead on indexed
//! arrays — the compound initializer, the element assignment and the
//! compound append all stored the value unconverted.
//!
//! GNU make_variable_value (variables.c:2975) applies CASE_CAPITALIZE
//! (sh_modcase: first character uppercased, the REST lowercased) on
//! every bind, and array elements bind individually
//! (make_array_variable_value, arrayfunc.c:166).
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
fn issue1619_capcase_indexed_initializer() {
    let (stdout, _, code) = rubash(r#"declare -ca a=(hello world); echo "init:${a[0]} ${a[1]}""#);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "init:Hello World\n");
}

#[test]
fn issue1619_capcase_indexed_initializer_lowercases_rest() {
    let (stdout, _, _) = rubash(r#"declare -ca a=(mCD eFG); echo "mixed:${a[0]} ${a[1]}""#);
    assert_eq!(stdout, "mixed:Mcd Efg\n");
}

#[test]
fn issue1619_capcase_indexed_element_assignment() {
    let (stdout, _, _) = rubash(r#"declare -ca b; b[0]=hello; echo "elem:${b[0]}""#);
    assert_eq!(stdout, "elem:Hello\n");
}

#[test]
fn issue1619_capcase_indexed_compound_append() {
    let (stdout, _, _) = rubash(r#"declare -ca c=(one); c+=(hello); echo "app:${c[0]} ${c[1]}""#);
    assert_eq!(stdout, "app:One Hello\n");
}

#[test]
fn issue1619_capcase_element_append_converts_concatenation() {
    // GNU converts the post-append value: stored "X" + appended "Y"
    // capcase("XY") = "Xy", not "X" + capcase("Y").
    let (stdout, _, _) = rubash(r#"declare -ca e=(x); e[0]+='Y'; echo "elemapp:${e[0]}""#);
    assert_eq!(stdout, "elemapp:Xy\n");
}

#[test]
fn issue1619_capcase_scalar_still_works() {
    let (stdout, _, _) = rubash(r#"declare -c s; s=hello; echo "scalar:$s""#);
    assert_eq!(stdout, "scalar:Hello\n");
}
