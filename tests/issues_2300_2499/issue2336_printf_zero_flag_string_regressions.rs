//! rubash#2336 — printf's `0` flag is defined only for numeric
//! conversions. GNU hands `%s`/`%c` to libc printf via the PF macro
//! (builtins/printf.def), where glibc ignores `0` for non-numeric
//! conversions, and routes `%b`/`%q`/`%Q` through printstr
//! (printf.def:861), which honors only `-` among the flags and always
//! pads with spaces. Numeric zero padding is unchanged.
//!
//! Expectations verified against GNU bash 5.3.20 (-c strings and
//! script files; outputs are pure ASCII apart from the `%c` NUL case).

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
fn printf_zero_flag_ignored_for_string_but_kept_for_numbers() {
    let (stdout, stderr, code) = rubash("printf '[%05d][%08.2f][%05s]\\n' -42 -3.5 hi");
    assert_eq!(stdout, "[-0042][-0003.50][   hi]\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn printf_zero_flag_ignored_for_char_and_b() {
    let (stdout, stderr, code) =
        rubash("printf '[%05c][%05.2s][%08.3s][%-05s][%05b][%0s]\\n' x hello hello hi hi hi");
    assert_eq!(stdout, "[    x][   he][     hel][hi   ][   hi][hi]\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn printf_zero_flag_ignored_with_space_and_plus_flags() {
    let (stdout, stderr, code) = rubash("printf '[% 05s][%+05s][%05q]\\n' hi hi hi");
    assert_eq!(stdout, "[   hi][   hi][   hi]\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn printf_char_zero_width_pads_nul_with_spaces() {
    let (stdout, stderr, code) = rubash("printf '[%05s][%05c][%0c][%0s]\\n' '' ''");
    assert_eq!(stdout, "[     ][    \u{0}][\u{0}][]\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
