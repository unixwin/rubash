//! rubash#2974 — `local -f` / `local -F` with no operands inside a
//! function printed the shell's function definitions; GNU prints the
//! function's local variables (usually none), never definitions.
//!
//! GNU mechanism (builtins/declare.def, declare_internal's no-operand
//! branch): `if (local_var) show_local_var_attributes (0, nodefs);` —
//! the local builtin's no-operand form IGNORES the attribute flags
//! entirely and lists the variables local to the current context
//! (builtins/setattr.def show_local_var_attributes walks
//! all_local_variables). -F only sets nodefs, which strips values in
//! the shared show_var_attributes rendering. Function definitions are
//! never reachable from this path.
//!
//! Expectations measured against GNU bash 5.3.20 (oracle binary),
//! script probes 2026-11-07.

use std::process::{Command, Stdio};
use std::time::Duration;

mod common;
use common::wait_bounded;

fn run_case(script: &str) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(["-c", script])
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    let out = wait_bounded(&mut child, Duration::from_secs(20));
    assert!(!out.timed_out, "case timed out: {script}");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.code,
    )
}

#[test]
fn issue2974_local_f_nameless_prints_nothing() {
    let (stdout, _, code) = run_case("f() { local -f; }; f; echo \"rc=$?\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0\n");
}

#[test]
fn issue2974_local_big_f_nameless_prints_nothing() {
    let (stdout, _, code) = run_case("f() { local -F; }; f; echo \"rc=$?\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0\n");
}

#[test]
fn issue2974_local_f_lists_locals_not_function_definitions() {
    // With a local in scope, `local -f` shows the local WITH its value
    // and `local -F` (nodefs) shows it bare — the flags never switch
    // the listing over to function definitions.
    let (stdout, _, _) =
        run_case("g() { :; }; f() { local lv=9; local -f; echo ---; local -F; }; f");
    assert_eq!(stdout, "declare -- lv=\"9\"\n---\ndeclare -- lv\n");
}

#[test]
fn issue2974_bare_local_still_lists_locals() {
    let (stdout, _, _) = run_case("f() { local lv=9; local; }; f");
    assert_eq!(stdout, "declare -- lv=\"9\"\n");
}
