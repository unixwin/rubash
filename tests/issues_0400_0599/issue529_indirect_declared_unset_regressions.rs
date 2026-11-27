//! Issue rubash#529 — `${!x}` on a declared-unset variable must expand
//! empty, not abort the script.
//!
//! GNU `parameter_brace_expand_indir` (subst.c:7883) fires the fatal
//! "invalid indirect expansion" only when `find_variable_last_nameref`
//! (variables.c:2051) returns NULL — no variable entry at all, or a nameref
//! chain stopped on an empty cell. A bare `declare x` / `local x` creates a
//! real (att_invisible) entry, so `parameter_brace_find_indir` yields a NULL
//! value and the expansion is EMPTY; under `set -u` the caller's unbound
//! check (subst.c:10164-10175) then reports `!x`:
//!
//! ```text
//! declare x; echo "[${!x}]"; echo done
//! ```
//!
//! GNU prints `[]` and `done` (rc 0); rubash used to abort with
//! `x: invalid indirect expansion` rc 1 before `done`.
//!
//! Every case below is byte-verified against WSL GNU Bash 5.3.0 (stdin
//! oracle, 2026-10-12); the set -u / invalid-target / nameref cells pin the
//! neighboring outcomes so the ordering cannot drift back.

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

/// #529 core: a global declared-unset variable expands empty and the script
/// continues (GNU probe: `[]` + `done`, rc 0).
#[test]
fn declared_unset_indirect_expands_empty() {
    let (stdout, stderr, code) = rubash("declare x; echo \"[${!x}]\"; echo done");
    assert_eq!(stdout, "[]\ndone\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// #529: a function-local declared-unset variable behaves the same (GNU
/// probe: `[]` + `done`, rc 0).
#[test]
fn local_declared_unset_indirect_expands_empty() {
    let (stdout, stderr, code) = rubash("f() { local x; echo \"[${!x}]\"; }; f; echo done");
    assert_eq!(stdout, "[]\ndone\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// #529 boundary: under `set -u` the declared-unset source reaches the
/// caller's unbound check instead — GNU reports `!x` (subst.c:10164-10175,
/// err_unboundvar(name) carries the whole `!x` body). The `-c` fatal exits
/// 127 via the run_one_command FORCE_EOF remap (shell.c:1471; WSL 5.3.0
/// probe: `bash -c 'set -u; declare x; echo ${!x}'` -> 127; script mode
/// exits 1).
#[test]
fn declared_unset_indirect_reports_bang_name_under_nounset() {
    let (stdout, stderr, code) = rubash("set -u; declare x; echo \"[${!x}]\"; echo done");
    assert_eq!(stdout, "");
    assert!(stderr.contains("!x: unbound variable"), "{stderr:?}");
    assert_eq!(code, Some(127));
}

/// #529 neighbor kept fatal: a never-set source still reports
/// "invalid indirect expansion" (find_variable_last_nameref == 0), before
/// any nounset check even under set -u.
#[test]
fn never_set_source_keeps_invalid_indirect_error() {
    let (stdout, stderr, _) = rubash("echo ${!x}; echo done");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("x: invalid indirect expansion"),
        "{stderr:?}"
    );
    let (stdout, stderr, _) = rubash("set -u; echo ${!x}; echo done");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("x: invalid indirect expansion"),
        "{stderr:?}"
    );
}

/// #529 neighbor kept fatal: an empty-cell nameref still reports
/// "invalid indirect expansion" (variables.c:2070 returns NULL for the
/// empty nameref cell even though the entry exists).
#[test]
fn empty_cell_nameref_keeps_invalid_indirect_error() {
    let (stdout, stderr, _) = rubash("declare -n n; echo \"[${!n}]\"; echo done");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("n: invalid indirect expansion"),
        "{stderr:?}"
    );
    let (stdout, stderr, _) = rubash("set -u; declare -n n; echo \"[${!n}]\"; echo done");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("n: invalid indirect expansion"),
        "{stderr:?}"
    );
}

/// #529 neighbor unchanged: a nameref whose cell names an absent variable
/// still expands to the CELL text (ksh93 nameref shortcut, subst.c:7892).
#[test]
fn resolving_nameref_cell_unchanged() {
    let (stdout, stderr, code) = rubash("declare -n r=x; echo \"[${!r}]\"; echo done");
    assert_eq!(stdout, "[x]\ndone\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    let (stdout, stderr, code) = rubash("set -u; declare -n r=x; echo \"[${!r}]\"; echo done");
    assert_eq!(stdout, "[x]\ndone\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// #529 ordering fix: under `set -u` a source value that is not a valid
/// indirect target reports the VALUE ("invalid variable name", raised
/// inside parameter_brace_expand_indir at subst.c:7955-7961), not `!x`.
#[test]
fn invalid_target_value_preempts_nounset_report() {
    let (stdout, stderr, _) = rubash("set -u; x=1bad; echo ${!x}; echo done");
    assert!(stderr.contains("1bad: invalid variable name"), "{stderr:?}");
    assert_eq!(stdout, "");
    let (stdout, stderr, _) = rubash("set -u; x=; echo ${!x}; echo done");
    assert!(stderr.contains(": invalid variable name"), "{stderr:?}");
    assert_eq!(stdout, "");
}

/// #529 neighbor unchanged: a valid source whose TARGET is unset still
/// reports `!x` under set -u (the agreed huck#54 boundary cell).
#[test]
fn valid_source_unset_target_reports_bang_name() {
    let (stdout, stderr, _) = rubash("set -u; x=qq; echo \"[${!x}]\"; echo done");
    assert!(stderr.contains("!x: unbound variable"), "{stderr:?}");
    assert_eq!(stdout, "");
}
