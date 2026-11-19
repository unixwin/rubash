//! rubash#3782 — scalar `declare -i` / `local -i` `+=` did not wrap at
//! INT64_MAX: it stored the out-of-range decimal (e.g. "9223372036854775808")
//! instead of wrapping to INT64_MIN the way GNU bash does.
//!
//! Root cause: the scalar integer-append arm added the two evalexp results
//! in i128 and stored the sum verbatim. GNU `make_variable_value`
//! (third_party/bash/variables.c:2905-2946) adds them as intmax_t and
//! stores via itos, so the sum wraps (expr.c:22: all arithmetic is intmax_t
//! with no overflow checking). The same un-narrowed sum also lived in the
//! assoc append helpers and the circular-nameref fallback; all of them now
//! go through `intmax_append_sum`.
//!
//! Expectations verified against GNU bash 5.3.20 (self-compiled oracle).
//! Checks 1-5 are the issue's regression seed; 6-10 are its controls.

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
fn scalar_declare_i_plus_eq_wraps_at_int64_max() {
    // 1: scalar -i += wraps MAX+1 to MIN.
    let (stdout, _, code) = rubash("declare -i x=9223372036854775807; x+=1; echo $x");
    assert_eq!(stdout, "-9223372036854775808\n", "stdout: {stdout}");
    assert_eq!(code, Some(0));

    // 2: scalar -i += wraps 5+MAX.
    let (stdout, _, _) = rubash("declare -i x=5; x+=9223372036854775807; echo $x");
    assert_eq!(stdout, "-9223372036854775804\n", "stdout: {stdout}");

    // 3: declare -p shows the wrapped stored value.
    let (stdout, _, _) = rubash("declare -i x=9223372036854775807; x+=1; declare -p x");
    assert_eq!(
        stdout, "declare -i x=\"-9223372036854775808\"\n",
        "stdout: {stdout}"
    );

    // 4: local -i += inside a function wraps too.
    let (stdout, _, _) = rubash("f() { local -i x=9223372036854775807; x+=1; echo $x; }; f");
    assert_eq!(stdout, "-9223372036854775808\n", "stdout: {stdout}");

    // 5: the wrapped stored value persists across plain reads.
    let (stdout, _, _) = rubash("declare -i x=9223372036854775807; x+=1; echo $x; echo $x");
    assert_eq!(
        stdout, "-9223372036854775808\n-9223372036854775808\n",
        "stdout: {stdout}"
    );
}

#[test]
fn scalar_declare_i_plus_eq_controls_still_wrap() {
    // 6 (control): (( x+=1 )) wraps.
    let (stdout, _, _) = rubash("declare -i x=9223372036854775807; (( x+=1 )); echo $x");
    assert_eq!(stdout, "-9223372036854775808\n", "stdout: {stdout}");

    // 7 (control): plain x=x+1 wraps.
    let (stdout, _, _) = rubash("declare -i x=9223372036854775807; x=x+1; echo $x");
    assert_eq!(stdout, "-9223372036854775808\n", "stdout: {stdout}");

    // 8 (control): $(( )) wraps.
    let (stdout, _, _) = rubash("echo $(( 9223372036854775807 + 1 ))");
    assert_eq!(stdout, "-9223372036854775808\n", "stdout: {stdout}");

    // 9 (control): array-element a[0]+=1 wraps.
    let (stdout, _, _) =
        rubash("declare -ai a; a[0]=9223372036854775807; a[0]+=1; echo \"${a[0]}\"");
    assert_eq!(stdout, "-9223372036854775808\n", "stdout: {stdout}");

    // 10 (control): a second += re-parses with wrap.
    let (stdout, _, _) = rubash("declare -i x=9223372036854775807; x+=1; x+=1; echo $x");
    assert_eq!(stdout, "-9223372036854775807\n", "stdout: {stdout}");
}

#[test]
fn declare_i_plus_eq_operand_and_negative_side_wrap() {
    // 11: `declare -i x+=1` as a single command wraps (the declare
    // integer-eval pass evaluates only the RHS; the append itself runs
    // through the same scalar arm).
    let (stdout, _, _) = rubash("declare -i x=9223372036854775807; declare -i x+=1; echo $x");
    assert_eq!(stdout, "-9223372036854775808\n", "stdout: {stdout}");

    // 12: `local -i y+=1` as a single command wraps.
    let (stdout, _, _) =
        rubash("f() { local -i y=9223372036854775807; local -i y+=1; echo $y; }; f");
    assert_eq!(stdout, "-9223372036854775808\n", "stdout: {stdout}");

    // 13: the negative side wraps too: MIN + (-1) is MAX.
    let (stdout, _, _) = rubash("declare -i y=-9223372036854775808; y+=-1; echo $y");
    assert_eq!(stdout, "9223372036854775807\n", "stdout: {stdout}");
}

#[test]
fn assoc_integer_append_wraps_at_int64_max() {
    // 14: whole-assoc scalar append onto key "0" wraps (same intmax_t
    // sum as the scalar arm).
    let (stdout, _, _) = rubash("declare -Ai a; a[0]=9223372036854775807; a+=1; echo \"${a[0]}\"");
    assert_eq!(stdout, "-9223372036854775808\n", "stdout: {stdout}");

    // 15: compound assoc `[k]+=1` append wraps.
    let (stdout, _, _) =
        rubash("declare -Ai b; b[k]=9223372036854775807; b+=( [k]+=1 ); echo \"${b[k]}\"");
    assert_eq!(stdout, "-9223372036854775808\n", "stdout: {stdout}");
}
