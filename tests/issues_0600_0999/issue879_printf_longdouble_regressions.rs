//! Issue rubash#879 regression: printf float operands convert through the
//! 80-bit extended (long double) format, and `%a` prints the 64-bit
//! mantissa nibble-style.
//!
//! GNU spec (WSL GNU bash 5.3 at /usr/local/bin/bash, script-file probes):
//! - builtins/printf.def:208-213 floatmax_t/strtofltmax (strtold),
//!   printf.def:774-798 float conversions via getfloatmax with the `L'
//!   modifier, printf.def:1387-1398 chk_converror (suffix beats ERANGE).
//! - `printf '%f' 1e999` expands the full ~1000-digit long-double value
//!   (f64 would print `inf`); `%a` of 5 is `0xap-1` (glibc `%La` style,
//!   not f64-style `0x1.4p+2`); decimal rounding breaks ties towards even.
//! - ERANGE (`Numerical result out of range`, status 1) fires on overflow
//!   (`1e9999` -> `inf`) and on inexact underflow (`1e-9999`,
//!   `1e-5000`, denormal `1e-4940`); a trailing suffix beats ERANGE.
//!
//! Rust semantic owners: src/builtins/printf/longdouble.rs (exact
//! decimal/hex -> 64-bit mantissa conversion, exact digit expansion),
//! src/builtins/printf/float.rs (exact %f/%e/%g/%a engines),
//! src/builtins/printf/number.rs (parse_long_double + ERANGE flag).

#![cfg(windows)]

use std::io::Write;
use std::process::{Command, Stdio};

fn run_script(script: &str) -> (Option<i32>, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(script.as_bytes())
        .expect("write script to stdin");
    let output = child.wait_with_output().expect("wait for rubash");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// `%f` of 1e999 is the full long-double expansion (1000 integer digits),
/// not f64 `inf`; no diagnostic, status 0.
#[test]
fn fixed_of_1e999_expands_long_double() {
    let (code, stdout, stderr) = run_script("printf '%f\\n' 1e999\n");
    assert_eq!(code, Some(0));
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");
    let rendered = stdout.trim_end_matches('\n');
    assert_eq!(rendered.len(), 1007, "1000 digits + .000000");
    assert!(rendered.starts_with("10000000000000000000300877674749523981268712005758"));
    assert!(rendered.ends_with("47344771885134373715968.000000"));
}

/// High-precision and exponent conversions read long-double digits.
#[test]
fn exp_and_precision_read_long_double_digits() {
    let (code, stdout, stderr) = run_script(
        "printf '%e\\n' 1e999\nprintf '%.50f\\n' 0.1\nprintf '%.17g\\n' 0.1\nprintf '%g\\n' 1e999\nprintf '%e\\n' 5e-324\n",
    );
    assert_eq!(code, Some(0));
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");
    // f64(0.1) prints ...5511151231257827 at this precision; the
    // long-double expansion prints ...13552527156088054250931600109.
    assert_eq!(
        stdout,
        "1.000000e+999\n\
         0.10000000000000000000135525271560688054250931600109\n\
         0.1\n\
         1e+999\n\
         5.000000e-324\n"
    );
}

/// `%a`/`%A` print the 64-bit mantissa nibble-style (glibc `%La`).
#[test]
fn hex_float_uses_mantissa_nibbles() {
    let (code, stdout, stderr) = run_script(
        "printf '%a\\n' 5\nprintf '%a\\n' 0.5\nprintf '%A\\n' 5\n\
         printf '%a\\n' 4.2\nprintf '%.2a\\n' 4.2\nprintf '%.20a\\n' 5\n\
         printf '%#a\\n' 5\nprintf '%.3a\\n' 0\nprintf '%a\\n' 1e10\n\
         printf '%a\\n' 5e-324\nprintf '%A\\n' 2.5\nprintf '%a\\n' 2.5\n",
    );
    assert_eq!(code, Some(0));
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");
    assert_eq!(
        stdout,
        "0xap-1\n\
         0x8p-4\n\
         0XAP-1\n\
         0x8.666666666666666p-1\n\
         0x8.66p-1\n\
         0xa.00000000000000000000p-1\n\
         0xa.p-1\n\
         0x0.000p+0\n\
         0x9.502f9p+30\n\
         0x8.18995ce7aa0e1b2p-1077\n\
         0XAP-2\n\
         0xap-2\n"
    );
}

/// Hex-float rounding is round-half-even on nibbles, including the
/// carry-past-leading-nibble renormalize.
#[test]
fn hex_float_rounding_breaks_ties_to_even() {
    let (code, stdout, stderr) = run_script(
        "printf '%.1a\\n' 0x1.78p0\nprintf '%.0a\\n' 0x1.9p0\n\
         printf '%.0a\\n' 0x1.bp0\nprintf '%.0a\\n' 0xF.8p4\n",
    );
    assert_eq!(code, Some(0));
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");
    // 0xC kept even on an exact half stays; 0xD odd rounds up; 0xF.8
    // carries all the way to 0x1p+8.
    assert_eq!(stdout, "0xb.cp-3\n0xcp-3\n0xep-3\n0x1p+8\n");
}

/// Decimal rounding breaks ties towards even on the exact value.
#[test]
fn decimal_rounding_breaks_ties_to_even() {
    let (code, stdout, stderr) = run_script(
        "printf '%.0f\\n' 0.5\nprintf '%.0f\\n' 1.5\nprintf '%.0f\\n' 2.5\n\
         printf '%.0f\\n' -2.5\nprintf '%.1f\\n' 0.15\nprintf '%.2g\\n' 99.9\n\
         printf '%.4g\\n' 9.9999\nprintf '%.0g\\n' 0.5\n",
    );
    assert_eq!(code, Some(0));
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");
    // 0.15 is exactly 0.1500000000000000000013... in long double, so the
    // guard digit past `0.1` is 5 with a nonzero tail: rounds up to 0.2.
    // %.2g of 99.9 rounds to 100 and flips to exponent style.
    assert_eq!(stdout, "0\n2\n2\n-2\n0.2\n1e+02\n10\n0.5\n");
}

/// Overflow raises ERANGE (`inf`, status 1); inexact underflow raises
/// ERANGE too; a trailing suffix beats ERANGE (plain invalid number).
#[test]
fn erange_overflow_underflow_and_suffix() {
    let (code, stdout, stderr) = run_script("printf '%f\\n' 1e9999\n");
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "inf\n");
    assert!(
        stderr.contains("printf: 1e9999: Numerical result out of range"),
        "{stderr}"
    );

    let (code, stdout, stderr) = run_script("printf '%f\\n' -1e-9999\n");
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "-0.000000\n");
    assert!(stderr.contains("Numerical result out of range"), "{stderr}");

    let (code, stdout, stderr) = run_script("printf '%f\\n' 1e9999x\n");
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "inf\n");
    assert!(
        stderr.contains("printf: 1e9999x: invalid number"),
        "{stderr}"
    );
    assert!(!stderr.contains("out of range"), "{stderr}");

    // Rounding the max finite long double up one ulp is infinity.
    let (code, stdout, stderr) = run_script("printf '%f\\n' 0xFFFFFFFFFFFFFFFF.8p16320\n");
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "inf\n");
    assert!(stderr.contains("Numerical result out of range"), "{stderr}");

    let (code, stdout, stderr) = run_script("printf '%f\\n' 0x1p16384\n");
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "inf\n");
    assert!(stderr.contains("Numerical result out of range"), "{stderr}");
}

/// The max finite long double itself formats finitely with no diagnostic.
#[test]
fn max_long_double_stays_finite() {
    let (code, stdout, stderr) = run_script("printf '%f\\n' 0xFFFFFFFFFFFFFFFFp16320\n");
    assert_eq!(code, Some(0));
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");
    let rendered = stdout.trim_end_matches('\n');
    assert!(
        rendered.starts_with("118973149535723176502"),
        "{rendered:.60}"
    );
    assert!(rendered.ends_with(".000000"));
}

/// Denormals format by the same nibble formula; inexact ones raise ERANGE
/// while an exact denormal does not.
#[test]
fn denormals_format_and_flag_inexact() {
    let (code, stdout, stderr) = run_script("printf '%a\\n' 1e-4940\n");
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "0x0.000000663278e62p-16385\n");
    assert!(stderr.contains("Numerical result out of range"), "{stderr}");

    let (code, stdout, stderr) = run_script("printf '%a\\n' 0x1p-16400\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "0x0.0002p-16385\n");
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");
}

/// PR #900 review (finding 1): a >100-significant-hex-digit literal kept
/// the 100-digit window but lost `4 * drop` of exponent compensation, so
/// the value came out 16^drop times too small. 101 hex digits here is
/// 16^100 + 1 = 2^400 + 1: `%f` expands 2^400 exactly, `%a` reads 0x8p+397.
#[test]
fn hex_window_truncation_compensates_exponent() {
    let literal = format!("0x1{}1p0", "0".repeat(99));
    let (code, stdout, stderr) = run_script(&format!(
        "printf '%a\\n' {literal}\nprintf '%f\\n' {literal}\n"
    ));
    assert_eq!(code, Some(0));
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");
    assert_eq!(
        stdout,
        "0x8p+397\n\
         2582249878086908589655919172003011874329705792829223512830659356540647622016841194629645353280137831435903171972747493376.000000\n"
    );
}

/// PR #900 review (finding 2): the hex underflow guard `4 * digits + pexp
/// <= -16445` zeroed the (2^-16446, 2^-16445) band an octave too early;
/// glibc rounds anything above half the minimum denormal up to it
/// (inexact -> ERANGE). The exact half still ties to even -> zero.
#[test]
fn hex_underflow_guard_stops_below_half_min_denormal() {
    for (operand, expected) in [
        ("0xFp-16449", "0x0.000000000000001p-16385\n"),
        ("0x9p-16449", "0x0.000000000000001p-16385\n"),
        ("0x8p-16449", "0x0p+0\n"),
        ("0x7p-16449", "0x0p+0\n"),
    ] {
        let (code, stdout, stderr) = run_script(&format!("printf '%a\\n' {operand}\n"));
        assert_eq!(code, Some(1), "{operand}");
        assert_eq!(stdout, expected, "{operand}");
        assert!(
            stderr.contains("Numerical result out of range"),
            "{operand}: {stderr}"
        );
    }
}

/// PR #900 review (finding 3): DECIMAL_KEEP_DIGITS = 32 truncated long
/// decimal literals below binary tie boundaries the true value sits at or
/// above (10 = 2 * 5 is not a power of two, so the dropped tail is not
/// bit-aligned and `sticky` cannot rescue a crossing). The widened window
/// keeps these literals on the exact path.
#[test]
fn decimal_window_preserves_tie_boundaries() {
    let (code, stdout, stderr) = run_script(
        "printf '%a\\n' 1.00000000000000000005421010862427523\n\
         printf '%.21f\\n' 1.00000000000000000005421010862427523\n\
         printf '%a\\n' 1.0000000000000000001626303258728256651011179201304912567138671875\n\
         printf '%.21f\\n' 1.0000000000000000001626303258728256651011179201304912567138671875\n",
    );
    assert_eq!(code, Some(0));
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");
    // First literal: just above the 1 + 2^-64 half-point. Second: the
    // exact half-point 1 + 3*2^-64, kept mantissa odd -> tie to even up.
    assert_eq!(
        stdout,
        "0x8.000000000000001p-3\n\
         1.000000000000000000108\n\
         0x8.000000000000002p-3\n\
         1.000000000000000000217\n"
    );
}

/// PR #900 review (finding 4): the '0' flag has no effect on a non-finite
/// result (glibc) — inf/nan pad with spaces and any sign stays adjacent
/// to the word.
#[test]
fn zero_flag_is_ignored_for_nonfinite() {
    let (code, stdout, stderr) = run_script(
        "printf '%010a\\n' nan\nprintf '%010a\\n' inf\n\
         printf '%010A\\n' NAN\nprintf '%015a\\n' nan\n\
         printf '%010f\\n' inf\nprintf '%010e\\n' nan\n\
         printf '%+010a\\n' nan\nprintf '%010a\\n' -nan\n",
    );
    assert_eq!(code, Some(0));
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");
    assert_eq!(
        stdout,
        "       nan\n\
         \x20      inf\n\
         \x20      NAN\n\
         \x20           nan\n\
         \x20      inf\n\
         \x20      nan\n\
         \x20     +nan\n\
         \x20     -nan\n"
    );
}

/// Zero padding goes after the sign and the 0x/0X prefix.
#[test]
fn zero_padding_follows_sign_and_hex_prefix() {
    let (code, stdout, stderr) = run_script(
        "printf '%015a\\n' 5\nprintf '%015A\\n' 5\nprintf '%+015a\\n' 5\n\
         printf '%015a\\n' -5\nprintf '%#010x\\n' 255\n",
    );
    assert_eq!(code, Some(0));
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");
    assert_eq!(
        stdout,
        "0x000000000ap-1\n0X000000000AP-1\n+0x00000000ap-1\n-0x00000000ap-1\n0x000000ff\n"
    );
}
