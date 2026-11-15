//! Issue rubash#3661 — `POSIXLY_CORRECT` assigned at runtime does not
//! switch `export -p` to POSIX form (modernish `isset -x` suite gate).
//!
//! GNU anchors (bash 5.3.20, vendored tree `~/workspace/rubash-audit/
//! build-gnu/bash-5.3/`):
//! - `builtins/setattr.def:504-510` (`show_var_attributes`):
//!   `posixly_correct == 0` prints `declare -%s `, else prints
//!   `%s ...` with `this_command_name` (i.e. `export ...`).
//! - `variables.c:5820` + `6257-6266` (`sv_strict_posix`): any bind of
//!   `POSIXLY_CORRECT` re-derives `posixly_correct`
//!   (`posixly_correct = var && var_isset (var)`) and calls
//!   `posix_initialize()` + `set_shellopts()`.
//! - `builtins/set.def:406-411` (`set_posix_mode`): `set -o posix` sets
//!   `posixly_correct` — the path rubash already had.
//!
//! Every expectation below was probed on the GNU 5.3.20 oracle with a
//! clean environment (`env -i`) before the fix was written. The probes
//! also settled two edge shapes: `mapfile POSIXLY_CORRECT` does NOT
//! flip POSIX mode on GNU (its array store bypasses the scalar-bind
//! hook), so this file carries no mapfile nail; compound, element,
//! append, `declare`/`export`/`readonly` array, arithmetic and
//! `printf -v` binds ALL flip it, so they are nailed here.

use std::process::Command;

fn rubash_c(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash -c");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Classify the `export -p` form of one variable via the shell itself
/// (builtins only, so the probe needs no external `grep`): emits
/// `POSIX` when the line reads `export NAME=...`, `DECLARE` when it
/// reads `declare -x NAME=...`.
const FORM_PROBE: &str = r#"
form_of() {
    out=$(export -p)
    case "$out" in
        *'export RUBASH3661_FOO="bar"'*) echo POSIX ;;
        *) echo DECLARE ;;
    esac
}
"#;

/// The issue repro: `export FOO=bar; POSIXLY_CORRECT=y; export -p`
/// must print the POSIX `export ...` form, not `declare -x ...`.
#[test]
fn runtime_assign_flips_export_p() {
    let (stdout, _, code) = rubash_c(&format!(
        "{FORM_PROBE}\nunset POSIXLY_CORRECT; export RUBASH3661_FOO=bar; POSIXLY_CORRECT=y; form_of"
    ));
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "POSIX", "stdout was: {stdout}");
}

/// An empty-but-set `POSIXLY_CORRECT=` still flips POSIX mode on
/// (GNU `var_isset`); the value itself stays empty — the hook must
/// not clobber it to "y" the way `set -o posix` binds it.
#[test]
fn empty_value_still_posix_and_value_preserved() {
    let (stdout, _, code) = rubash_c(&format!(
        "{FORM_PROBE}\nunset POSIXLY_CORRECT; export RUBASH3661_FOO=bar; POSIXLY_CORRECT=; form_of; declare -p POSIXLY_CORRECT"
    ));
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("POSIX"),
        "expected POSIX form, stdout was: {stdout}"
    );
    assert!(
        stdout.contains("declare -- POSIXLY_CORRECT=\"\""),
        "empty value must survive the hook, stdout was: {stdout}"
    );
}

/// `unset POSIXLY_CORRECT` takes the `var == 0` branch: POSIX mode off.
#[test]
fn unset_flips_back() {
    let (stdout, _, code) = rubash_c(&format!(
        "{FORM_PROBE}\nunset POSIXLY_CORRECT; export RUBASH3661_FOO=bar; POSIXLY_CORRECT=y; unset POSIXLY_CORRECT; form_of"
    ));
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "DECLARE", "stdout was: {stdout}");
}

/// `set -o posix` keeps binding `POSIXLY_CORRECT="y"` (set.def:413).
#[test]
fn set_o_posix_binds_y() {
    let (stdout, _, code) = rubash_c(&format!(
        "{FORM_PROBE}\nunset POSIXLY_CORRECT; export RUBASH3661_FOO=bar; set -o posix; form_of; declare -p POSIXLY_CORRECT"
    ));
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("POSIX"),
        "expected POSIX form, stdout was: {stdout}"
    );
    assert!(
        stdout.contains("declare -- POSIXLY_CORRECT=\"y\""),
        "set -o posix must bind POSIXLY_CORRECT=y, stdout was: {stdout}"
    );
}

/// `set +o posix` unbinds `POSIXLY_CORRECT` and turns the mode off.
#[test]
fn set_plus_o_posix_unbinds() {
    let (stdout, _, code) = rubash_c(&format!(
        "{FORM_PROBE}\nunset POSIXLY_CORRECT; export RUBASH3661_FOO=bar; POSIXLY_CORRECT=y; set +o posix; form_of; declare -p POSIXLY_CORRECT"
    ));
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("DECLARE"),
        "expected DECLARE form, stdout was: {stdout}"
    );
    assert!(
        stdout.contains("not found"),
        "POSIXLY_CORRECT must be unbound after set +o posix, stdout was: {stdout}"
    );
}

/// A temporary prefix assignment flips POSIX mode for the command's
/// duration and the restore flips it back — no session leak.
#[test]
fn temp_prefix_flips_during_and_restores_after() {
    let (stdout, _, code) = rubash_c(
        "unset POSIXLY_CORRECT; export RUBASH3661_FOO=bar
POSIXLY_CORRECT=y export -p
echo ---AFTER---
export -p",
    );
    assert_eq!(code, Some(0));
    let (during, after) = stdout
        .split_once("---AFTER---")
        .expect("marker line in stdout");
    assert!(
        during.contains("export RUBASH3661_FOO=\"bar\""),
        "during the temp command the form must be POSIX, stdout was: {stdout}"
    );
    assert!(
        after.contains("declare -x RUBASH3661_FOO=\"bar\""),
        "after the temp command the form must be DECLARE again, stdout was: {stdout}"
    );
}

/// `export POSIXLY_CORRECT=y` is a bind: the hook fires.
#[test]
fn export_builtin_assign() {
    let (stdout, _, code) = rubash_c(&format!(
        "{FORM_PROBE}\nunset POSIXLY_CORRECT; export RUBASH3661_FOO=bar; export POSIXLY_CORRECT=y; form_of"
    ));
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "POSIX", "stdout was: {stdout}");
}

/// `declare POSIXLY_CORRECT=y` is a bind: the hook fires.
#[test]
fn declare_assign() {
    let (stdout, _, code) = rubash_c(&format!(
        "{FORM_PROBE}\nunset POSIXLY_CORRECT; export RUBASH3661_FOO=bar; declare POSIXLY_CORRECT=y; form_of"
    ));
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "POSIX", "stdout was: {stdout}");
}

/// A compound array bind of the name flips it too (GNU probe A).
#[test]
fn declare_array_compound_assign() {
    let (stdout, _, code) = rubash_c(&format!(
        "{FORM_PROBE}\nunset POSIXLY_CORRECT; export RUBASH3661_FOO=bar; declare -a POSIXLY_CORRECT=(a b); form_of"
    ));
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "POSIX", "stdout was: {stdout}");
}

/// An element bind of the name flips it too (GNU probe B).
#[test]
fn element_assign() {
    let (stdout, _, code) = rubash_c(&format!(
        "{FORM_PROBE}\nunset POSIXLY_CORRECT; export RUBASH3661_FOO=bar; POSIXLY_CORRECT[0]=y; form_of"
    ));
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "POSIX", "stdout was: {stdout}");
}

/// Appending to an unset name creates it — the hook fires (GNU probe E).
#[test]
fn append_assign() {
    let (stdout, _, code) = rubash_c(&format!(
        "{FORM_PROBE}\nunset POSIXLY_CORRECT; export RUBASH3661_FOO=bar; POSIXLY_CORRECT+=z; form_of"
    ));
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "POSIX", "stdout was: {stdout}");
}

/// `readonly POSIXLY_CORRECT=y` binds, then marks: the hook fires.
#[test]
fn readonly_assign() {
    let (stdout, _, code) = rubash_c(&format!(
        "{FORM_PROBE}\nunset POSIXLY_CORRECT; export RUBASH3661_FOO=bar; readonly POSIXLY_CORRECT=y; form_of"
    ));
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "POSIX", "stdout was: {stdout}");
}

/// A function-local bind flips POSIX mode inside the function and
/// the frame pop restores it afterwards — no session leak.
#[test]
fn local_in_function_restores() {
    let (stdout, _, code) = rubash_c(&format!(
        "{FORM_PROBE}\nunset POSIXLY_CORRECT; export RUBASH3661_FOO=bar
f() {{ local POSIXLY_CORRECT=y; form_of; }}
f
form_of"
    ));
    assert_eq!(code, Some(0));
    let mut lines = stdout.lines();
    assert_eq!(lines.next().unwrap_or(""), "POSIX", "inside f: {stdout}");
    assert_eq!(lines.next().unwrap_or(""), "DECLARE", "after f: {stdout}");
}

/// Arithmetic assignment is a bind: `(( POSIXLY_CORRECT=1 ))` flips it.
#[test]
fn arith_assign() {
    let (stdout, _, code) = rubash_c(&format!(
        "{FORM_PROBE}\nunset POSIXLY_CORRECT; export RUBASH3661_FOO=bar; (( POSIXLY_CORRECT=1 )); form_of"
    ));
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "POSIX", "stdout was: {stdout}");
}

/// Arithmetic element assignment flips it too.
#[test]
fn arith_element_assign() {
    let (stdout, _, code) = rubash_c(&format!(
        "{FORM_PROBE}\nunset POSIXLY_CORRECT; export RUBASH3661_FOO=bar; (( POSIXLY_CORRECT[0]=1 )); form_of"
    ));
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "POSIX", "stdout was: {stdout}");
}

/// `printf -v POSIXLY_CORRECT` is a bind: the hook fires.
#[test]
fn printf_v_assign() {
    let (stdout, _, code) = rubash_c(&format!(
        "{FORM_PROBE}\nunset POSIXLY_CORRECT; export RUBASH3661_FOO=bar; printf -v POSIXLY_CORRECT '%s' y; form_of"
    ));
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "POSIX", "stdout was: {stdout}");
}

/// `printf -v 'POSIXLY_CORRECT[0]'` (element bind) flips it too.
#[test]
fn printf_v_element_assign() {
    let (stdout, _, code) = rubash_c(&format!(
        "{FORM_PROBE}\nunset POSIXLY_CORRECT; export RUBASH3661_FOO=bar; printf -v 'POSIXLY_CORRECT[0]' '%s' y; form_of"
    ));
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "POSIX", "stdout was: {stdout}");
}

/// `read POSIXLY_CORRECT` is a bind: the hook fires.
#[test]
fn read_assign() {
    let (stdout, _, code) = rubash_c(&format!(
        "{FORM_PROBE}\nunset POSIXLY_CORRECT; export RUBASH3661_FOO=bar; read POSIXLY_CORRECT <<< \"y\"; form_of"
    ));
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "POSIX", "stdout was: {stdout}");
}

/// The modernish `isset -x` shape: the assignment runs inside a
/// command substitution (so the flip is visible there) and the
/// parent shell is untouched afterwards.
#[test]
fn comsub_isolation() {
    let (stdout, _, code) = rubash_c(&format!(
        "{FORM_PROBE}\nunset POSIXLY_CORRECT; export RUBASH3661_FOO=bar
inside=$(POSIXLY_CORRECT=y; export -p)
case \"$inside\" in *'export RUBASH3661_FOO=\"bar\"'*) echo INSIDE-POSIX ;; *) echo INSIDE-DECLARE ;; esac
form_of"
    ));
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("INSIDE-POSIX"),
        "inside the comsub the form must be POSIX, stdout was: {stdout}"
    );
    assert!(
        stdout.trim_end().ends_with("DECLARE"),
        "outside the comsub the form must stay DECLARE, stdout was: {stdout}"
    );
}

/// The production shape from modernish `isset -x`: `export -p` output
/// is eval'd with `export` aliased to a handler, and the handler must
/// run for every exported variable once POSIX mode is on.
#[test]
fn alias_handler_shape() {
    let (stdout, _, code) = rubash_c(
        "unset POSIXLY_CORRECT; export RUBASH3661_FOO=bar
shopt -s expand_aliases
handler() { echo \"HANDLER:$1\"; }
alias export=handler
eval \"$(POSIXLY_CORRECT=y; command export -p)\"
unalias export",
    );
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("HANDLER:RUBASH3661_FOO=bar"),
        "the aliased export handler must run for the variable, stdout was: {stdout}"
    );
}
