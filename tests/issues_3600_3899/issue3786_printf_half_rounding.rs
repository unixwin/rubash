//! rubash#3786: `printf %f` rounds exact-half decimal inputs down where
//! GNU rounds them up.
//!
//! GNU parses printf float operands with `strtold` and formats the long
//! double (builtins/printf.def:209 `floatmax_t`, :213 `strtofltmax`,
//! :1495-1515 `getfloatmax`, :774-798 `L`-modified conversions), so the
//! value is the 80-bit nearest to the decimal literal and the rendering
//! rounds that exact binary value once. rubash emulates the same long
//! double (`src/builtins/printf/longdouble.rs`), but `%f` dropped the
//! exact tail past the guard digit before rounding, so a value sitting
//! slightly ABOVE the decimal half (1.2345 -> 1.2345000000000000000026)
//! rounded as an exact tie and stayed down.
//!
//! Cells 1-4 are the defect (fail-first on the unfixed base); the rest
//! are controls that agree on both shells, including %e/%g spot checks
//! proving the other engines already fold the tail correctly.

use std::process::Command;

fn run(script: &str) -> (String, String, Option<i32>) {
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

fn check(script: &str, expected: &str) {
    let (stdout, stderr, status) = run(script);
    assert_eq!(stdout, expected, "script: {script} stderr: {stderr:?}");
    assert_eq!(status, Some(0), "script: {script} stderr: {stderr:?}");
    assert!(stderr.is_empty(), "script: {script} stderr: {stderr:?}");
}

#[test]
fn issue3786_printf_f3_1_2345_rounds_up() {
    // Long double 1.2345 sits slightly ABOVE the decimal half.
    check("printf '%.3f\\n' 1.2345", "1.235\n");
}

#[test]
fn issue3786_printf_f3_neg_1_234567_rounds_up() {
    // 1.234567 is past the 1.2345 midpoint, not a tie at all.
    check("printf '%.3f\\n' -1.234567", "-1.235\n");
}

#[test]
fn issue3786_printf_f2_1_245_rounds_up() {
    // Long double 1.245 sits slightly ABOVE the decimal half.
    check("printf '%.2f\\n' 1.245", "1.25\n");
}

#[test]
fn issue3786_printf_f3_width_neg_rounds_up() {
    check("printf '[%10.3f]\\n' -1.234567", "[    -1.235]\n");
}

#[test]
fn issue3786_printf_f2_1_235_control() {
    // Long double 1.235 sits slightly ABOVE 1.235: rounds up either way
    // (the unfixed code reached 1.24 via the tie-to-even kept-odd rule).
    check("printf '%.2f\\n' 1.235", "1.24\n");
}

#[test]
fn issue3786_printf_f2_2_675_control() {
    // Long double 2.675 sits slightly BELOW: guard digit 4 rounds down.
    check("printf '%.2f\\n' 2.675", "2.67\n");
}

#[test]
fn issue3786_printf_f1_0_15_control() {
    check("printf '%.1f\\n' 0.15", "0.2\n");
}

#[test]
fn issue3786_printf_f0_2_5_exact_tie_to_even() {
    // 2.5 is exactly representable: a true tie, kept digit even stays.
    check("printf '%.0f\\n' 2.5", "2\n");
}

#[test]
fn issue3786_printf_f4_1_23455_control() {
    check("printf '%.4f\\n' 1.23455", "1.2346\n");
}

#[test]
fn issue3786_printf_f3_1_2344_control() {
    check("printf '%.3f\\n' 1.2344", "1.234\n");
}

#[test]
fn issue3786_printf_f2_1_234567_control() {
    check("printf '%.2f\\n' 1.234567", "1.23\n");
}

#[test]
fn issue3786_printf_f5_1_234567_control() {
    check("printf '%.5f\\n' 1.234567", "1.23457\n");
}

#[test]
fn issue3786_printf_f6_default_precision_control() {
    check("printf '%f\\n' 1.234567", "1.234567\n");
}

#[test]
fn issue3786_printf_e2_1_245_control() {
    // %e already folds the tail (significant_digits sticky): must stay.
    check("printf '%.2e\\n' 1.245", "1.25e+00\n");
}

#[test]
fn issue3786_printf_g4_1_2345_control() {
    // %g already folds the tail: 4 significant digits round up.
    check("printf '%.4g\\n' 1.2345", "1.235\n");
}

#[test]
fn issue3786_printf_g3_1_2345_control() {
    // 3 significant digits: guard digit 4 rounds down.
    check("printf '%.3g\\n' 1.2345", "1.23\n");
}

#[test]
fn issue3786_printf_f3_0_0005_control() {
    // Long double 0.0005 sits slightly BELOW: rounds down.
    check("printf '%.3f\\n' 0.0005", "0.000\n");
}

#[test]
fn issue3786_printf_f3_999_9995_control() {
    // Long double 999.9995 sits slightly BELOW: rounds down.
    check("printf '%.3f\\n' 999.9995", "999.999\n");
}

#[test]
fn issue3786_printf_e2_1234_567_control() {
    check("printf '%.2e\\n' 1234.567", "1.23e+03\n");
}

#[test]
fn issue3786_printf_g3_1234_567_control() {
    check("printf '%.3g\\n' 1234.567", "1.23e+03\n");
}
