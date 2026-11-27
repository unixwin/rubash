//! rubash#1162 / #1164 / #1165 / #1166 — `return` inside trap
//! actions.
//!
//! GNU trap.c / execute_cmd.c semantics pinned here:
//! - #1162: a DEBUG trap whose action is `return 2`, with functrace
//!   (set -T) and a function call in flight — at top level the return
//!   is an error ("can only `return' from a function or sourced
//!   script"), the function body does not run its command, and the
//!   caller's `$?` after the call is 2.
//! - #1164: `return 3` in an ERR trap action RETURNS from the function
//!   whose command failed (status 3); the rest of the body is skipped.
//! - #1165: the function's own fall-off-end status survives its
//!   RETURN trap action in both directions: a body ending in `false`
//!   returns 1 (a printing trap runs, status stays 1), and a body
//!   ending in `true` returns 0 even when the trap action itself
//!   runs `false` — the action does not clobber the result.
//! - #1166: the return-in-trap diagnostic cites the line of the
//!   command that triggered the trap, and execution continues.
//!
//! All expectations verified against GNU bash 5.3.20 (`-c` strings);
//! diagnostics are matched on their stable cores (line prefixes carry
//! the binary's own path and the -c line numbering).

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
fn issue1162_debug_trap_return_at_top_level() {
    let (stdout, stderr, code) = rubash(
        "set -T\n\
         trap 'return 2' DEBUG\n\
         f() { echo in-f; }\n\
         f\n\
         echo after rc=$?\n\
         echo end",
    );
    assert_eq!(stdout, "after rc=2\nend\n");
    assert_eq!(
        stderr
            .matches("return: can only `return' from a function or sourced script")
            .count(),
        3,
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue1164_err_trap_return_returns_from_function() {
    let (stdout, _, code) = rubash(
        "f() { trap 'echo ERRTRAP; return 3' ERR; false; echo inner; }\nf\necho outer-rc=$?",
    );
    assert_eq!(stdout, "ERRTRAP\nouter-rc=3\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1165_return_trap_status_interplay() {
    let (stdout, _, code) = rubash(
        "f() { trap 'echo RETTRAP' RETURN; false; }; f; echo rc=$?\n\
         g() { trap 'false' RETURN; true; }; g; echo rc=$?",
    );
    assert_eq!(stdout, "RETTRAP\nrc=1\nrc=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1166_return_in_debug_trap_cites_triggering_line() {
    let (stdout, stderr, code) = rubash("trap 'if true; then return 1; fi' DEBUG\necho hi");
    assert_eq!(stdout, "hi\n");
    assert!(
        stderr.contains("line 2: return: can only `return' from a function or sourced script"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}
