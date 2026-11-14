//! rubash#3029 — in diagnostics, a command name whose bytes are
//! not printable IN THE ACTIVE LOCALE is rendered with GNU
//! ansic_quote (strtrans.c:230; gated by ansic_shouldquote,
//! strtrans.c:341): in a non-UTF-8 locale every byte >= 0x80 is
//! unprintable and octal-escaped inside `$'...'`; in a UTF-8
//! locale a printable wide char stays raw (ansic_wshouldquote).
//! rubash printed the raw UTF-8 bytes in every locale.
//! Fixtures are the eco-2906 mvdan/sh snippets t0084/t0227/t0228.
//! Expectations measured against GNU bash 5.3.20 (2026-11-08).

use std::path::PathBuf;
use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_file(tag: &str, script: &str) -> Run {
    run_file_env(tag, script.as_bytes(), &[])
}

fn run_file_env(tag: &str, script: &[u8], extra_env: &[(&str, &str)]) -> Run {
    let dir: PathBuf = std::env::temp_dir().join(format!("rubashP1-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    std::fs::write(dir.join("case.sh"), script).expect("write fixture");
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .current_dir(&dir)
        .arg("case.sh")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &dir);
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let output = command.output().expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn assert_run(tag: &str, run: &Run, stdout: &str, stderr: &str, code: i32) {
    assert_eq!(run.stdout, stdout, "{tag}: stdout");
    assert_eq!(run.stderr, stderr, "{tag}: stderr");
    assert_eq!(run.code, Some(code), "{tag}: rc");
}

#[test]
fn issue3029_non_ascii_assignment_name_octal_quoted_c_locale() {
    // t0084: `à=b foo` — à is U+00E0, UTF-8 bytes C3 A0.
    let mut script = vec![0xC3, 0xA0];
    script.extend_from_slice(b"=b foo\n");
    let run = run_file_env("t0084-c", &script, &[("LC_ALL", "C")]);
    assert_run(
        "t0084-c",
        &run,
        "",
        "case.sh: line 1: $'\\303\\240=b': command not found\n",
        127,
    );
}

#[test]
fn issue3029_non_ascii_assignment_name_raw_utf8_locale() {
    // Same bytes under a UTF-8 locale: à is a printable wide char,
    // so GNU prints the name raw — the quoting is locale-gated.
    let mut script = vec![0xC3, 0xA0];
    script.extend_from_slice(b"=b foo\n");
    let run = run_file_env("t0084-u8", &script, &[("LC_ALL", "C.UTF-8")]);
    assert_run(
        "t0084-u8",
        &run,
        "",
        "case.sh: line 1: \u{00e0}=b: command not found\n",
        127,
    );
}

#[test]
fn issue3029_dollar_non_ascii_name_octal_quoted_c_locale() {
    // t0228: `$à` — the whole name including `$` is quoted.
    let mut script = b"$".to_vec();
    script.extend_from_slice(&[0xC3, 0xA0]);
    script.push(b'\n');
    let run = run_file_env("t0228-c", &script, &[("LC_ALL", "C")]);
    assert_run(
        "t0228-c",
        &run,
        "",
        "case.sh: line 1: $'$\\303\\240': command not found\n",
        127,
    );
}

#[test]
fn issue3029_mixed_dollar_names_octal_quoted_c_locale() {
    // t0227: `$àb $àb $,b` — `$` before a byte that cannot start a
    // name stays literal, so the command name is the whole `$àb` and
    // the quoting covers the `$` too (GNU bash 5.3.20 re-probed
    // 2026-11-09 on these exact bytes; an earlier draft of this pin
    // dropped the `$` from the expectation, which the oracle does
    // not do).
    let mut script = b"$".to_vec();
    script.extend_from_slice(&[0xC3, 0xA0]);
    script.extend_from_slice(b"b $");
    script.extend_from_slice(&[0xC3, 0xA0]);
    script.extend_from_slice(b"b $,b\n");
    let run = run_file_env("t0227-c", &script, &[("LC_ALL", "C")]);
    assert_run(
        "t0227-c",
        &run,
        "",
        "case.sh: line 1: $'$\\303\\240b': command not found\n",
        127,
    );
}

#[test]
fn issue3029_ascii_name_control() {
    let run = run_file("ascii", "zzznotfound\n");
    assert_run(
        "ascii",
        &run,
        "",
        "case.sh: line 1: zzznotfound: command not found\n",
        127,
    );
}
