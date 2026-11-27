//! Regression pins for rubash#1061 and rubash#1242 (regex match atom
//! granularity; `let` diagnostic byte shape).
//!
//! #1061: in `[[ "aé" =~ a(.) ]]`, the `.` atom captured the FULL char
//! (bytes c3 a9). GNU bash 5.3 matches with the C library regex in the
//! current locale; in the C locale (and in the issue's Termux oracle env)
//! `.` consumes ONE BYTE, so BASH_REMATCH[1] is the single byte c3 —
//! an incomplete UTF-8 sequence GNU happily hands back. `.*` captures
//! agreed on both shells; only single-unit atom granularity diverged.
//! Issue repro, oracle-checked against GNU bash 5.3.20:
//!   [[ "aé" =~ a(.) ]]; printf '%s' "${BASH_REMATCH[1]}" | od -An -tx1
//!   GNU: ` c3`   old niu: ` c3 a9`
//! The pin runs under LC_ALL=C so the byte semantics are the locale's
//! explicit contract on every CI host.
//!
//! #1242: bare `let` printed `let: expression expected` WITHOUT the
//! trailing newline on stderr (verified with od -c against GNU bash
//! 5.3.20: the diagnostic is newline-terminated, exit 1). `let ""` is
//! silent rc 1 on both; `let "1/0"` diagnostics matched byte-for-byte —
//! only the no-argument path dropped the `\n`.

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    rubash_env(script, &[])
}

fn rubash_env(script: &str, envs: &[(&str, &str)]) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg("-c").arg(script);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

#[test]
fn issue1061_regex_dot_captures_one_byte_in_c_locale() {
    let (stdout, _stderr, _code) = rubash_env(
        "[[ \"aé\" =~ a(.) ]]; printf '%s' \"${BASH_REMATCH[1]}\" | od -An -tx1 | tr -d ' \\n'",
        &[("LC_ALL", "C")],
    );
    assert_eq!(
        stdout.trim(),
        "c3",
        "single `.` atom must capture exactly one byte (c3), got {stdout:?}"
    );
}

#[test]
fn issue1242_bare_let_diagnostic_is_newline_terminated() {
    let (_stdout, stderr, code) = rubash("let");
    assert_eq!(code, Some(1), "bare let exits 1");
    assert!(
        stderr.contains("let: expression expected"),
        "bare let diagnostic text, got: {stderr:?}"
    );
    assert!(
        stderr.ends_with('\n'),
        "bare let diagnostic must be newline-terminated, got: {stderr:?}"
    );
}

#[test]
fn issue1242_let_empty_string_silent_and_divzero_terminated() {
    // Control legs from the issue: both already agreed with GNU and must
    // not be disturbed by the newline fix.
    let (_o, stderr, code) = rubash("let \"\"");
    assert_eq!(code, Some(1), "let \"\" exits 1");
    assert!(stderr.is_empty(), "let \"\" is silent, got: {stderr:?}");
    let (_o2, stderr2, code2) = rubash("let \"1/0\"");
    assert_eq!(code2, Some(1), "let 1/0 exits 1");
    assert!(
        stderr2.contains("let: 1/0: division by 0 (error token is \"0\")")
            && stderr2.ends_with('\n'),
        "let 1/0 diagnostic shape (GNU wording is `division by 0`), got: {stderr2:?}"
    );
}
