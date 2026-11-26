//! rubash#1285 (W5 residual / characterization pin): `printf %a` / `%A`
//! are ACCEPTED and print C99 hex-float form, rc=0.
//!
//! Ruling (do not "fix"): the on-device report came from an Android
//! (bionic) GNU build, whose printf rejects `%a` as an invalid format
//! character. This project's sole baseline is glibc/WSL GNU bash 5.3
//! (repo AGENTS.md baseline rule), and glibc GNU ACCEPTS `%a`/`%A` —
//! the hex-float output below is byte-identical to the glibc GNU
//! 5.3.20 oracle. Ruled 2026-10-24: "维持基线规矩，不修" (issue closing
//! comment; ~/workspace/rubash-fix3/DECISIONS.md: "rubash #1285 →
//! 维持不修：仓库 AGENTS.md 基线规矩已定，bionic 分歧不实现").
//! The bionic split is a recorded oracle difference, not a bug: on an
//! Android-built GNU the same script errors, and that divergence is
//! accepted deliberately. Related: rubash#879 compares the %a mantissa
//! style against the WSL (glibc) oracle, which is the governing one.

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

#[test]
fn issue1285_printf_percent_a_accepted_glibc_form() {
    let (stdout, stderr, status) = run("printf '%a\\n' 3.14; echo \"rc=$?\"");
    assert_eq!(
        stdout, "0xc.8f5c28f5c28f5c3p-2\nrc=0\n",
        "stderr: {stderr:?}"
    );
    assert_eq!(status, Some(0), "stderr: {stderr:?}");
}

#[test]
fn issue1285_printf_percent_capital_a_accepted_glibc_form() {
    let (stdout, stderr, status) = run("printf '%A\\n' 3.14; echo \"rc=$?\"");
    assert_eq!(
        stdout, "0XC.8F5C28F5C28F5C3P-2\nrc=0\n",
        "stderr: {stderr:?}"
    );
    assert_eq!(status, Some(0), "stderr: {stderr:?}");
}

#[test]
fn issue1285_printf_percent_a_exact_binary_fraction() {
    // GNU bash's own %a normalization: 0.5 prints as `0x8p-4` under the
    // glibc oracle (verified against GNU bash 5.3.20), not the C
    // library's `0x1p-1` form.
    let (stdout, stderr, status) = run("printf '%a\\n' 0.5; echo \"rc=$?\"");
    assert_eq!(stdout, "0x8p-4\nrc=0\n", "stderr: {stderr:?}");
    assert_eq!(status, Some(0), "stderr: {stderr:?}");
}
