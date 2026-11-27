//! rubash#203 / rubash#204 — Syntax errors in sourced files vs the
//! main script, and the offending line's quoted text.
//!
//! GNU parse.y / builtins/source.def:
//! - #203: a syntax error inside a SOURCED file makes `.` return 2;
//!   the rest of the sourced file is abandoned (its later lines do
//!   not run) but the OUTER script CONTINUES and its own exit status
//!   is unaffected. The error itself is reported against the sourced
//!   file and line, quoting the raw line text (`` `echo)' `` for the
//!   line `echo)`).
//! - #204: the same error in the MAIN script is fatal: rc=2, later
//!   lines do not run, and the quote reproduces the line as written,
//!   spaces included (`` `echo )' `` for the line `echo )`).
//!
//! Expectations byte-verified against GNU bash 5.3.20 with SCRIPT
//! FILES (fixtures built in a tempdir).

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash203-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run(tag: &str, files: &[(&str, &str)], main: &str) -> (String, String, Option<i32>) {
    let dir = tempdir(tag);
    for (name, body) in files {
        std::fs::write(dir.join(name), body).expect("write fixture");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg(main)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue203_sourced_syntax_error_returns_2_and_outer_continues() {
    let (stdout, stderr, code) = run(
        "sourced",
        &[
            ("inner.sh", "echo inner-start\necho)\necho inner-end\n"),
            ("outer.sh", "echo A\n. ./inner.sh\necho \"rc=$?\"\necho B\n"),
        ],
        "outer.sh",
    );
    assert_eq!(stdout, "A\ninner-start\nrc=2\nB\n");
    assert!(
        stderr.contains("inner.sh: line 2: syntax error near unexpected token `)'"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("`echo)'"), "stderr: {stderr}");
    assert_eq!(code, Some(0), "the outer script's status is its own");
}

#[test]
fn issue204_main_script_syntax_error_is_fatal() {
    let (stdout, stderr, code) = run(
        "main",
        &[("probe.sh", "echo t1\necho )\necho t3\n")],
        "probe.sh",
    );
    assert_eq!(stdout, "t1\n", "t3 must not run");
    assert!(
        stderr.contains("line 2: syntax error near unexpected token `)'"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("`echo )'"),
        "the line is quoted as written, space included; stderr: {stderr}"
    );
    assert_eq!(code, Some(2));
}
