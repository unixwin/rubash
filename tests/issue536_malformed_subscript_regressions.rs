//! Issue rubash#536 — malformed array subscripts on the READ path must
//! report, not expand silently empty.
//!
//! GNU validates the `${...}` reference before the operator dispatch
//! (subst.c:10053 parameter_brace_expand -> valid_brace_expansion_word,
//! subst.c:7583): an empty subscript (`${x[]}`) or a second subscript
//! (`${a[0][1]}`) fails valid_array_reference (arrayfunc.c:885 — the
//! subscript must be non-empty and simply bracketed) and reports
//! `${...}: bad substitution` (subst.c:10278) with status 1, abandoning
//! the command while the script continues. A failing subscript
//! EXPRESSION (`${a[$((1/0))]}`) raises the arithmetic expansion error
//! instead (`1/0: division by 0 (error token is "0")`, expr.c evalerror
//! via arrayfunc.c:1353 array_expand_index -> evalexp). rubash expanded
//! all three to the empty string with rc 0 — the guard script never saw
//! the malformed data.
//!
//! Every case is byte-verified against WSL GNU Bash 5.3.0 (script-file
//! oracle, 2026-10-12). The assignment path was already conformant
//! (huck#703/572 ledger) and stays pinned.

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let path = std::env::temp_dir().join(format!("rubash536_{}.sh", std::process::id()));
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// #536 repro 1: an empty subscript reports bad substitution naming the
/// whole `${...}` text; the command dies, the next line runs.
#[test]
fn empty_subscript_read_reports_bad_substitution() {
    let (stdout, stderr, code) = rubash("echo ${x[]}\necho after");
    assert_eq!(stdout, "after\n");
    assert!(stderr.contains("${x[]}: bad substitution"), "{stderr:?}");
    assert_eq!(code, Some(0));
    // Same class with the variable set and on assoc arrays.
    let (stdout, stderr, _) = rubash("x=5; echo ${x[]}\necho after");
    assert_eq!(stdout, "after\n");
    assert!(stderr.contains("${x[]}: bad substitution"), "{stderr:?}");
    let (stdout, stderr, _) = rubash("declare -A A; echo ${A[]}\necho after");
    assert_eq!(stdout, "after\n");
    assert!(stderr.contains("${A[]}: bad substitution"), "{stderr:?}");
}

/// #536 repro 2: a double subscript reports bad substitution.
#[test]
fn double_subscript_read_reports_bad_substitution() {
    let (stdout, stderr, _) = rubash("a=(1 2 3); echo ${a[0][1]}\necho after");
    assert_eq!(stdout, "after\n");
    assert!(
        stderr.contains("${a[0][1]}: bad substitution"),
        "{stderr:?}"
    );
    let (stdout, stderr, _) = rubash("echo ${a[0][1]:1}\necho after");
    assert_eq!(stdout, "after\n");
    assert!(
        stderr.contains("${a[0][1]:1}: bad substitution"),
        "{stderr:?}"
    );
}

/// #536 repro 3 (D4): a failing subscript expression raises the
/// arithmetic error with the expr.c message shape and abandons the
/// command.
#[test]
fn failing_subscript_arithmetic_reports_division_by_zero() {
    let (stdout, stderr, _) = rubash("echo ${a[$((1/0))]}\necho after");
    assert_eq!(stdout, "after\n");
    assert!(
        stderr.contains("1/0: division by 0 (error token is \"0\")"),
        "{stderr:?}"
    );
}

/// #536: the indirect and length prefixes obey the same reference rule
/// (GNU validates the reference before the indir/length dispatch).
#[test]
fn indirect_and_length_empty_subscripts_report() {
    let (stdout, stderr, _) = rubash("echo ${!x[]}\necho after");
    assert_eq!(stdout, "after\n");
    assert!(stderr.contains("${!x[]}: bad substitution"), "{stderr:?}");
    let (stdout, stderr, _) = rubash("echo ${#x[]}\necho after");
    assert_eq!(stdout, "after\n");
    assert!(stderr.contains("${#x[]}: bad substitution"), "{stderr:?}");
}

/// #536 boundary kept: every well-formed subscript shape keeps expanding —
/// negative indices, @/*, element substring, arithmetic, nested-subscript
/// arithmetic, and operator tails after a valid subscript.
#[test]
fn valid_subscript_forms_keep_expanding() {
    let (stdout, _, code) =
        rubash("x=(1 2 3); echo ${x[-1]} ${x[@]} ${x[0]:1} ${x[$((1+1))]}; echo ok");
    assert_eq!(stdout, "3 1 2 3 3\nok\n");
    assert_eq!(code, Some(0));
    let (stdout, _, _) = rubash("i=(0); x=(1 2 3); echo ${x[$((i[0]+1))]}; echo ok");
    assert_eq!(stdout, "2\nok\n");
    // Same-line list: GNU's bad substitution abandons the rest of the
    // current command list, so the `echo rc=$?` tail never runs; the next
    // LINE still runs.
    let (stdout, stderr, _) = rubash("echo ${x[]/#a/b}; echo rc=$?");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("${x[]/#a/b}: bad substitution"),
        "{stderr:?}"
    );
    let (stdout, stderr, _) = rubash("echo ${x[]/#a/b}\necho rc=$?");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("${x[]/#a/b}: bad substitution"),
        "{stderr:?}"
    );
}

/// #536 boundary kept: the ASSIGNMENT path already reports (ledger
/// huck#703/572) — pinned so the read-path fix cannot mask it.
#[test]
fn assignment_path_still_reports() {
    let (stdout, stderr, _) = rubash("a[1/0]=v\necho after");
    assert!(stderr.contains("division by 0"), "{stderr:?}");
    assert_eq!(stdout, "after\n");
}
