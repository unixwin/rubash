//! rubash#1101 — invalid octal constant in a variable VALUE (`z=08`)
//! silently evaluated as decimal.
//!
//! GNU expr.c:1539-1557 strlong: a variable value with a leading zero is
//! read in the octal arm ("0nnn -> base 8"); a digit 8/9 there is
//! `value too great for base`, an expansion error that aborts the
//! non-interactive shell with status 1. The literal form (`$((08))`)
//! already errored; the value path took a plain-decimal shortcut in
//! `evaluate_variable_text` and printed 8 with rc=0.
//!
//! All expectations byte-verified against GNU bash 5.3.20 (script files).

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
fn invalid_octal_variable_value_aborts_expansion() {
    let (stdout, stderr, code) = rubash("z=08; echo $((z)); echo rc=$?");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("08: value too great for base (error token is \"08\")"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(1));
}

#[test]
fn signed_invalid_octal_values_error_with_unsigned_token() {
    for value in ["-08", "+09"] {
        let (stdout, stderr, code) = rubash(&format!("z={value}; echo $((z))"));
        assert_eq!(stdout, "", "value {value}");
        assert!(
            stderr.contains("value too great for base"),
            "value {value} stderr: {stderr}"
        );
        assert_eq!(code, Some(1), "value {value}");
    }
}

#[test]
fn invalid_octal_value_in_command_contexts_reports_and_continues() {
    // `(( ))` / `let` are command contexts: the diagnostic carries the
    // command label and the shell keeps running with status 1.
    let (stdout, stderr, code) = rubash("z=08; ((z)); echo rc=$?; let z; echo rc2=$?");
    assert_eq!(stdout, "rc=1\nrc2=1\n");
    assert!(
        stderr.contains("((: 08: value too great for base"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("let: 08: value too great for base"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn valid_octal_and_decimal_values_still_evaluate() {
    let (stdout, stderr, code) =
        rubash("a=010; b=123; c=0; d=-010; echo $((a)) $((b)) $((c)) $((d))");
    assert_eq!(stdout, "8 123 0 -8\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
