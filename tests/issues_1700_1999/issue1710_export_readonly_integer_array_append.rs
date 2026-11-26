//! rubash#1710 — `export` / `readonly` with the append form `name+=value`
//! on an integer indexed or associative array string-concatenated the
//! operand text onto element `0` instead of performing GNU's arithmetic
//! append. GNU routes the bind through `bind_variable` with ASS_APPEND
//! into `make_variable_value` (variables.c:2905), whose integer branch
//! stores `itos (evalexp (old element 0) + evalexp (operand))`.
//!
//! Expectations verified against GNU bash 5.3.20 (self-compiled oracle),
//! both as a script file and as `-c` strings. The scalar-append, plain
//! (non-append) bind, and non-integer concat lines are controls that
//! already agreed with GNU and must not regress.

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
fn issue1710_export_integer_indexed_append_adds() {
    let (stdout, _, code) = rubash("declare -ia f=(10); export 'f+=5'; echo \"Q:${f[0]}\"");
    assert_eq!(stdout, "Q:15\n", "stdout: {stdout}");
    assert_eq!(code, Some(0));
    // Only element 0 is appended to; the remaining elements survive.
    let (stdout, _, _) =
        rubash("declare -ia ac=(1 20); export 'ac+=5'; echo \"${ac[0]},${ac[1]}\"");
    assert_eq!(stdout, "6,20\n", "stdout: {stdout}");
}

#[test]
fn issue1710_readonly_integer_indexed_append_evaluates_operand() {
    let (stdout, _, _) = rubash("declare -ia e=(1); readonly 'e+=2*3'; echo \"N:${e[0]}\"");
    assert_eq!(stdout, "N:7\n", "stdout: {stdout}");
    let (stdout, _, _) = rubash("declare -ia s=(7); readonly 's+=-2'; echo \"${s[0]}\"");
    assert_eq!(stdout, "5\n", "stdout: {stdout}");
    let (stdout, _, _) = rubash("declare -ia big=(100); readonly 'big+=7%3'; echo \"${big[0]}\"");
    assert_eq!(stdout, "101\n", "stdout: {stdout}");
}

#[test]
fn issue1710_integer_assoc_append_adds_at_key_zero() {
    let (stdout, _, _) = rubash("declare -iA m=([0]=1); export 'm+=2*3'; echo \"O:${m[0]}\"");
    assert_eq!(stdout, "O:7\n", "stdout: {stdout}");
    let (stdout, _, _) = rubash("declare -iA n=([0]=10); readonly 'n+=5'; echo \"${n[0]}\"");
    assert_eq!(stdout, "15\n", "stdout: {stdout}");
    // A missing key 0 starts from 0; the other keys are untouched.
    let (stdout, _, _) = rubash("declare -iA z=([k]=3); export 'z+=2*3'; echo \"${z[0]}|${z[k]}\"");
    assert_eq!(stdout, "6|3\n", "stdout: {stdout}");
}

#[test]
fn issue1710_append_operand_is_a_full_arithmetic_expression() {
    // Variable references in the operand resolve (GNU evalexp).
    let (stdout, _, _) = rubash("declare -ia g=(1); x=4; export 'g+=x'; echo \"R:${g[0]}\"");
    assert_eq!(stdout, "R:5\n", "stdout: {stdout}");
    // Parenthesised subexpressions evaluate.
    let (stdout, _, _) = rubash("declare -ia t=(3); export 't+=2*(1+2)'; echo \"${t[0]}\"");
    assert_eq!(stdout, "9\n", "stdout: {stdout}");
    // Self-reference reads the prior element 0 on both array kinds.
    let (stdout, _, _) = rubash("declare -ia v=(2); export 'v+=v[0]'; echo \"${v[0]}\"");
    assert_eq!(stdout, "4\n", "stdout: {stdout}");
    let (stdout, _, _) = rubash("declare -iA q=([0]=2); export 'q+=q[0]'; echo \"${q[0]}\"");
    assert_eq!(stdout, "4\n", "stdout: {stdout}");
    // Octal literals follow arithmetic rules, not decimal text.
    let (stdout, _, _) = rubash("declare -ia ab=(8); export 'ab+=010'; echo \"${ab[0]}\"");
    assert_eq!(stdout, "16\n", "stdout: {stdout}");
}

#[test]
fn issue1710_unset_array_and_quoted_compound_text_shapes() {
    // Declared-but-unset integer array: element 0 materializes as 0+operand.
    let (stdout, _, _) = rubash("declare -ia u; export 'u+=4'; echo \"${u[0]}|${#u[@]}\"");
    assert_eq!(stdout, "4|1\n", "stdout: {stdout}");
    // A quoted "(5)" operand is literal text for setattr builtins, so the
    // integer append evaluates it as the parenthesised expression 5 and no
    // new element appears.
    let (stdout, _, _) = rubash("declare -ia c=(1); export 'c+=(5)'; echo \"${c[0]},${c[1]}\"");
    assert_eq!(stdout, "6,\n", "stdout: {stdout}");
    // Negative results keep their sign.
    let (stdout, _, _) = rubash("declare -ia neg=(5); export 'neg+=-9'; echo \"${neg[0]}\"");
    assert_eq!(stdout, "-4\n", "stdout: {stdout}");
}

#[test]
fn issue1710_controls_scalar_plain_and_noninteger_unchanged() {
    // Scalar integer append through export already agreed.
    let (stdout, _, _) = rubash("declare -i k=1; export 'k+=2*3'; echo \"C:$k\"");
    assert_eq!(stdout, "C:7\n", "stdout: {stdout}");
    // Plain (non-append) binds onto element 0 already agreed.
    let (stdout, _, _) = rubash("declare -ia d=(1 2); export 'd=2*3'; echo \"${d[0]},${d[1]}\"");
    assert_eq!(stdout, "6,2\n", "stdout: {stdout}");
    let (stdout, _, _) = rubash("declare -iA p=([k]=1); export 'p=2*3'; echo \"${p[0]}\"");
    assert_eq!(stdout, "6\n", "stdout: {stdout}");
    // A non-integer array append stays a string concatenation.
    let (stdout, _, _) = rubash("declare -a w=(ab); export 'w+=cd'; echo \"${w[0]}\"");
    assert_eq!(stdout, "abcd\n", "stdout: {stdout}");
}
