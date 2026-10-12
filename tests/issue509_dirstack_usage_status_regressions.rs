//! Issue rubash#509 regression: the dirstack builtins' numeric-argument
//! validation is EX_USAGE (2) in GNU, not a plain builtin failure (1) — and
//! `dirs -l +q` must not silently consume the operand (rubash printed the
//! stack and exited 0).
//!
//! GNU spec (WSL GNU bash 5.3.0 probes, 2026-10-12, all stdin-driven):
//! builtins/common.c get_numeric_arg -> `%s: invalid number` + usage,
//! return EX_USAGE(=2); dirs.def's non-flag word is `invalid option`,
//! popd's non-± word is `invalid argument` — all rc 2.
//!
//! ```text
//! $ printf 'dirs +x'    | bash -s; echo $?   # 2  dirs: +x: invalid number
//! $ printf 'dirs -Q'    | bash -s; echo $?   # 2  dirs: -Q: invalid number
//! $ printf 'popd +x'    | bash -s; echo $?   # 2  popd: +x: invalid number
//! $ printf 'pushd +q'   | bash -s; echo $?   # 2  pushd: +q: invalid number
//! $ printf 'dirs -l +q' | bash -s; echo $?   # 2  dirs: +q: invalid number
//! ```

#![cfg(windows)]

use std::io::Write;
use std::process::{Command, Stdio};

fn run_stdin(script: &str) -> (Option<i32>, String) {
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
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The issue table: every numeric-validation failure exits 2 with GNU's
/// diagnostic shape.
#[test]
fn issue_table_validation_failures_exit_2() {
    let cases: [(&str, &str); 8] = [
        ("dirs +x", "dirs: +x: invalid number"),
        ("dirs -Q", "dirs: -Q: invalid number"),
        ("popd +x", "popd: +x: invalid number"),
        ("pushd +q", "pushd: +q: invalid number"),
        ("dirs -l +q", "dirs: +q: invalid number"),
        ("dirs foo", "dirs: foo: invalid option"),
        ("popd 3", "popd: 3: invalid argument"),
        ("pushd -n +q", "pushd: +q: invalid number"),
    ];
    for (script, needle) in cases {
        let (code, stderr) = run_stdin(&format!("{script}\n"));
        assert_eq!(code, Some(2), "rc for {script:?}");
        assert!(
            stderr.contains(needle),
            "diagnostic for {script:?}: {stderr:?}"
        );
        assert!(
            stderr.contains("usage:"),
            "usage line for {script:?}: {stderr:?}"
        );
    }
}

/// The silent half of the issue: `dirs -l +q` used to print the stack and
/// exit 0 — nothing on stdout now.
#[test]
fn dirs_l_plus_q_prints_nothing() {
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
        .write_all(b"dirs -l +q\n")
        .expect("write script to stdin");
    let output = child.wait_with_output().expect("wait for rubash");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty(), "silent pass leaked output");
}

/// The valid forms are untouched (GNU all rc 0).
#[test]
fn valid_forms_still_succeed() {
    // Listing forms on the initial one-entry stack.
    for script in [
        "dirs\n",
        "dirs -l\n",
        "dirs -p\n",
        "dirs -v\n",
        "dirs -l -v\n",
    ] {
        let (code, stdout) = {
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
            )
        };
        assert_eq!(code, Some(0), "rc for {script:?}");
        assert!(!stdout.trim().is_empty(), "no listing for {script:?}");
    }

    // Index display: `dirs +0`/`dirs -v +0` address element 0; an out-of
    // range index stays a plain failure (1), not a usage error.
    let (code, _) = run_stdin("dirs +0\n");
    assert_eq!(code, Some(0));
    let (code, _) = run_stdin("dirs -v +0\n");
    assert_eq!(code, Some(0));
    let (code, _) = run_stdin("dirs -v +9\n");
    assert_eq!(code, Some(1));

    // pushd/popd round trip and `dirs -c`.
    let (code, _) = run_stdin("pushd /\n");
    assert_eq!(code, Some(0));
    let (code, _) = run_stdin("pushd /\npopd +0\n");
    assert_eq!(code, Some(0));
    let (code, _) = run_stdin("dirs -c\n");
    assert_eq!(code, Some(0));
}
