//! Issue rubash#519 — caller must not fabricate a main frame past the
//! actual call stack.
//!
//! GNU builtins/caller.def (caller_builtin:75-121) reads FUNCNAME /
//! BASH_SOURCE / BASH_LINENO arrays; `caller EXPR` needs BASH_LINENO[EXPR],
//! FUNCNAME[EXPR+1] and BASH_SOURCE[EXPR+1], and any absent element fails
//! with EXECUTION_FAILURE (status 1, no output). The synthetic bottom
//! "main" frame exists ONLY when the shell reads a script FILE
//! (shell.c:1636-1648 open_shell_script pushes funcname_a "main");
//! `bash -s`/`bash -c` never enter open_shell_script, so at g→f depth
//! `caller 1` and `caller 2` are both empty with status 1. Rubash padded
//! FUNCNAME with "main" unconditionally, inventing a `10 main <path>`
//! line with status 0.
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0 via the
//! stdin harness (printf '%s' | wsl bash -s, 2026-10-12).

use std::io::Write;
use std::process::{Command, Stdio};

/// Run `script` through rubash -s (stdin script mode, the issue's harness).
fn rubash_stdin(script: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-s")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(script.as_bytes())
        .expect("write script");
    let output = child.wait_with_output().expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

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

/// The issue's table, stdin harness: at g→f depth `caller 1` and
/// `caller 2` are both empty with rc=1 (GNU: `rc=1` twice, no output).
#[test]
fn caller_beyond_stack_is_empty_with_status_one() {
    let script = "f() {\n\
                  caller 1; echo \"rc1=$?\"\n\
                  caller 2; echo \"rc2=$?\"\n\
                  }\n\
                  g() { f; }\n\
                  g\n";
    let (stdout, _stderr, _code) = rubash_stdin(script);
    assert_eq!(stdout, "rc1=1\nrc2=1\n");
}

/// In-stack forms keep working (GNU: `caller 0` → `9 g <file>`, rc=0;
/// `caller 0 99` ignores the second argument, rc=0).
#[test]
fn caller_in_stack_forms_still_report() {
    let script = "f() {\n\
                  caller 0 >/dev/null; echo \"rc0=$?\"\n\
                  caller 0 99 >/dev/null; echo \"rc0b=$?\"\n\
                  }\n\
                  g() { f; }\n\
                  g\n";
    let (stdout, _stderr, _code) = rubash_stdin(script);
    assert_eq!(stdout, "rc0=0\nrc0b=0\n");
}

/// `caller foo` keeps the invalid-number usage failure (GNU rc=2).
#[test]
fn caller_invalid_number_is_usage() {
    let (_stdout, stderr, code) = rubash("f() { caller foo; echo \"rc=$?\"; }; f");
    assert!(stderr.contains("caller: foo: invalid number"));
    assert!(stderr.contains("caller: usage: caller [expr]"));
    assert_eq!(code, Some(0));
    // rc of the CALLER builtin itself, printed by the script:
    let (stdout, _stderr, _code) = rubash("f() { caller foo >/dev/null 2>&1; echo \"rc=$?\"; }; f");
    assert_eq!(stdout, "rc=2\n");
}

/// -c mode has no main frame either (GNU `bash -c`: FUNCNAME has no
/// "main" entry — dynamic_arrays.rs has_script_main_frame is false).
#[test]
fn caller_beyond_stack_in_c_mode_is_empty() {
    let (stdout, _stderr, _code) = rubash("f() { caller 1; echo \"rc1=$?\"; }; g() { f; }; g");
    assert_eq!(stdout, "rc1=1\n");
}

/// Guard: at top level caller still fails like GNU (no function frame).
#[test]
fn caller_at_top_level_fails() {
    let (stdout, _stderr, _code) = rubash("caller 0; echo \"rc=$?\"");
    assert_eq!(stdout, "rc=1\n");
}
