//! Issue rubash#3582 (#3522 residual): `unset 'a[~]'` on an indexed
//! array must report the arithmetic diagnostic token `~`; rubash
//! printed the tilde expansion `/home/<user>` (error token is
//! "/home/...").
//!
//! GNU oracle (bash 5.3.20, vendored `third_party/bash/bash`, probed
//! 2026-11-14): `array_expand_index` (arrayfunc.c:1353-1368) resolves an
//! indexed subscript through `expand_arith_string` (subst.c:3981) —
//! parameter, command and arithmetic expansion only — so a leading `~`
//! reaches `evalexp` verbatim and `expr.c`'s operand-expected diagnostic
//! echoes it raw. The ASSOCIATIVE path is different:
//! `unbind_array_element`'s assoc arm (arrayfunc.c:1241-1251) resolves
//! the key through `expand_subscript_string` (subst.c:11063 ->
//! `expand_string` -> `expand_word_internal`), which DOES tilde-expand a
//! leading unquoted `~` — `unset 'h[~]'` on `declare -A h=([$HOME]=v)`
//! removes that key.
//!
//! Rubash ran the indexed subscript through the same assoc-grade
//! `expand_subscript_string` (`resolve_array_subscript`), whose trailing
//! `~`-prefix step tilde-expanded before `evalexp`. Fix: the indexed
//! evaluator resolves through an arith-grade pass
//! (`resolve_indexed_subscript` -> `expand_subscript_string_arith`) that
//! skips the tilde step; assoc consumers keep the old resolve.

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.arg0("bash");
    }
    let output = command
        .arg("-c")
        .arg(script)
        .env("LC_ALL", "C.UTF-8")
        .env("HOME", "/tmp/rubash3582home")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The issue repro: the operand-expected diagnostic must carry the raw
/// `~`, never the expanded HOME.
#[test]
fn unset_indexed_tilde_subscript_reports_raw_tilde() {
    let (stdout, stderr, rc) = rubash(r#"a=(x y z); unset "a[~]"; echo after"#);
    assert!(
        stderr.contains(r#"arithmetic syntax error: operand expected (error token is "~")"#),
        "expected GNU's raw-tilde diagnostic, got stderr={stderr:?}"
    );
    assert!(
        !stderr.contains("/tmp/rubash3582home"),
        "tilde leaked into the diagnostic: stderr={stderr:?}"
    );
    // GNU array_expand_index -> evalexp failure: expok==0 ->
    // jump_to_top_level(DISCARD) — the rest of the -c line never runs and
    // the shell exits 1 (probed on the vendored bash 5.3.20).
    assert!(stdout.is_empty(), "stdout={stdout:?}");
    assert_eq!(rc, Some(1));
}

/// The same subscript spelling reaches the indexed evaluator through
/// assignment and parameter read sites; all must print `~` verbatim.
#[test]
fn indexed_tilde_subscript_assignment_and_read_report_raw_tilde() {
    // The assignment's evalexp failure aborts the -c line (same DISCARD
    // jump), so only the first diagnostic appears and the read never runs
    // — identical on GNU 5.3.20.
    let (stdout, stderr, rc) = rubash(r#"a=(x y z); a[~]=9; echo "${a[~]}""#);
    assert_eq!(
        stderr
            .matches(r#"arithmetic syntax error: operand expected (error token is "~")"#)
            .count(),
        1,
        "stderr={stderr:?}"
    );
    assert!(!stderr.contains("/tmp/rubash3582home"), "stderr={stderr:?}");
    assert!(stdout.is_empty(), "stdout={stdout:?}");
    assert_eq!(rc, Some(1));
}

/// Associative subscripts keep the assoc-grade resolve: GNU's
/// `expand_subscript_string` DOES tilde-expand a leading unquoted `~`,
/// so `unset 'h[~]'` removes the `$HOME` key.
#[test]
fn assoc_tilde_subscript_still_tilde_expands() {
    let (stdout, _stderr, _rc) =
        rubash(r#"declare -A h=(["/tmp/rubash3582home"]=v [k]=w); unset "h[~]"; declare -p h"#);
    assert_eq!(stdout, "declare -A h=([k]=\"w\" )\n", "stdout={stdout:?}");
}
