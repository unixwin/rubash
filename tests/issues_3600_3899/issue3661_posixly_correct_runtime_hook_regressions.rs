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
//! hook), so this file carries no mapfile nail; compound, statement
//! element, append, `declare`/`export`/`readonly` array, arithmetic
//! scalar and `printf -v` scalar binds ALL flip it, so they are
//! nailed here.
//!
//! Four cells carry corrections against the first-draft nail, all
//! re-probed on the oracle: `set_plus_o_posix_unbinds` reads the
//! `not found` diagnostic on stderr with status 1 (not stdout/0), and
//! the temp-prefix restore cell uses `command export -p` because a
//! prefix on the special builtin `export` is promoted by GNU
//! (merge_temporary_env) instead of restored — that promotion is
//! nailed separately in `temp_prefix_special_builtin_persists`. The
//! arithmetic-element and `printf -v` element cells expected a flip,
//! but GNU keys the post-bind hook on the WRITTEN lhs/operand text
//! (`expr.c:352`, `builtins/printf.def:112-113`), so an element
//! spelling matches no special variable and does NOT flip; both cells
//! now expect DECLARE. (A plain statement element assignment and
//! `declare`/`local` element operands DO flip — those routes reach
//! the hook with the bare variable name, `variables.c:5314` /
//! `builtins/declare.def:1105`; `export`/`readonly`/`read` element
//! operands do not flip, oracle-verified.)

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
/// (Pin correction, GNU-oracle verified: `declare -p` on the unbound
/// name reports `not found` on STDERR and the command status is 1 —
/// the original cell read the wrong stream and expected status 0.)
#[test]
fn set_plus_o_posix_unbinds() {
    let (stdout, stderr, code) = rubash_c(&format!(
        "{FORM_PROBE}\nunset POSIXLY_CORRECT; export RUBASH3661_FOO=bar; POSIXLY_CORRECT=y; set +o posix; form_of; declare -p POSIXLY_CORRECT"
    ));
    assert_eq!(code, Some(1));
    assert!(
        stdout.contains("DECLARE"),
        "expected DECLARE form, stdout was: {stdout}"
    );
    assert!(
        stderr.contains("not found"),
        "POSIXLY_CORRECT must be unbound after set +o posix, stderr was: {stderr}"
    );
}

/// A temporary prefix assignment flips POSIX mode for the command's
/// duration and the restore flips it back — no session leak. The
/// command is `command export -p`: `command` is a regular builtin,
/// so the prefix stays temporary. (Pin correction, GNU-oracle
/// verified: with the special builtin `export` itself as the command,
/// GNU PROMOTES the binding — see the next cell — so the original
/// `POSIXLY_CORRECT=y export -p` shape cannot show a restore.)
#[test]
fn temp_prefix_flips_during_and_restores_after() {
    let (stdout, _, code) = rubash_c(
        "unset POSIXLY_CORRECT; export RUBASH3661_FOO=bar
POSIXLY_CORRECT=y command export -p
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

/// The special-builtin twin of the temp-prefix shape: GNU
/// merge_temporary_env (variables.c:4485-4525 push_posix_temp_var,
/// called only in posix mode) promotes an assignment preceding a
/// posix special builtin into the current execution environment.
/// The temp bind itself switches posix mode on, so
/// `POSIXLY_CORRECT=y export -p` leaves the variable bound and the
/// mode on afterwards (GNU-oracle verified).
#[test]
fn temp_prefix_special_builtin_persists() {
    let (stdout, _, code) = rubash_c(
        "unset POSIXLY_CORRECT; export RUBASH3661_FOO=bar
POSIXLY_CORRECT=y export -p
echo ---AFTER---
export -p
declare -p POSIXLY_CORRECT",
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
        after.contains("export RUBASH3661_FOO=\"bar\""),
        "after a special-builtin temp command the form must stay POSIX, stdout was: {stdout}"
    );
    assert!(
        after.contains("declare -x POSIXLY_CORRECT=\"y\""),
        "the promoted binding must survive, stdout was: {stdout}"
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

/// Arithmetic element assignment does NOT flip it: GNU's
/// expr_bind_variable keys the post-bind hook on the written lhs text
/// (`expr.c:352`), and `POSIXLY_CORRECT[0]` matches no special
/// variable. (Pin correction, GNU-oracle verified.)
#[test]
fn arith_element_assign() {
    let (stdout, _, code) = rubash_c(&format!(
        "{FORM_PROBE}\nunset POSIXLY_CORRECT; export RUBASH3661_FOO=bar; (( POSIXLY_CORRECT[0]=1 )); form_of"
    ));
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "DECLARE", "stdout was: {stdout}");
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

/// `printf -v 'POSIXLY_CORRECT[0]'` (element bind) does NOT flip it:
/// GNU keys the post-bind hook on the -v operand as written
/// (`builtins/printf.def:112-113`), subscript included. (Pin
/// correction, GNU-oracle verified.)
#[test]
fn printf_v_element_assign() {
    let (stdout, _, code) = rubash_c(&format!(
        "{FORM_PROBE}\nunset POSIXLY_CORRECT; export RUBASH3661_FOO=bar; printf -v 'POSIXLY_CORRECT[0]' '%s' y; form_of"
    ));
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "DECLARE", "stdout was: {stdout}");
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
