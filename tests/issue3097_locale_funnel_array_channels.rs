//! Regression pins for rubash#3097 — three sibling gaps in the
//! shared sv_locale funnel family, split out of #3065 at captain
//! review of PR #3080 (the three cells marked "moved" below are the
//! original #3080 draft cells with their assertions intact):
//!
//! Residual 1 — the element-assignment channel never fires the
//! funnel for an array locale variable: `LC_ALL[0]=bogus_XX` stores
//! through apply_shell_assignment_inner's element arm, which
//! returns via apply_nameref_array_element_assignment before the
//! function tail's note_locale_assignment, so no bind event ever
//! reaches the funnel. GNU's assignment driver fires
//! stupidly_hack_special_variables after assign_array_element just
//! as after bind_variable (subst.c:8169), reading the NEW scalar
//! projection (variables.c:2495 get_string_value).
//!
//! Residual 2 — the same store layer's scalar-over-array arms: a
//! scalar bind onto an existing indexed array (element 0) or assoc
//! ("0" key) returns early from apply_shell_assignment_inner
//! without the tail note, so `for LC_ALL in ...` over an array
//! variable and a plain `LC_ALL=bogus_XX` over an array both stay
//! silent. GNU's for loop binds with bind_variable and then runs
//! the hack (execute_cmd.c:3093 / :3097-3100); the bind lands in
//! element 0 (variables.c:3128-3142) and the funnel reads it.
//!
//! Residual 3 — the declare channel's funnel projection judges
//! array-ness differently from the scalar read path: the funnel's
//! stored_scalar_value (src/locale.rs) consults VarTable::attr_of,
//! while Executor::scalar_parameter_value consults the ARRAY_VARS /
//! ASSOC_VARS mark sets that declare-family writes maintain — so a
//! declare compound bind's warning names the raw storage render
//! (`\x1d([0]="bogus_XX")`, `(([0]=bogus_XX))`) instead of the
//! projection, and an assoc bind with no "0" key warns where GNU's
//! NULL projection takes the silent reset path.
//!
//! The declare fire COUNT (GNU fires twice for a declare compound
//! bind) is #3066's face and is deliberately not pinned here: the
//! declare cells assert naming only, except the no-"0"-key assoc
//! shape whose GNU-correct outcome is total silence.
//!
//! All expectations are the literal GNU bash 5.3.20 outputs for the
//! same script files in a locale-clean environment (only C/POSIX
//! locales installed; the bogus names can never resolve). Warning
//! errno suffixes follow glibc: only the first failed lookup of a
//! given locale name per process carries `: No such file or
//! directory`; a different bogus name earns its own suffix.
#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3097-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Base command with the whole locale family removed from the
/// inherited environment so each case starts from a known state.
fn base() -> Command {
    let mut cmd = Command::new(BIN);
    for var in [
        "LANG",
        "LC_ALL",
        "LC_COLLATE",
        "LC_CTYPE",
        "LC_MESSAGES",
        "LC_NUMERIC",
        "LC_TIME",
    ] {
        cmd.env_remove(var);
    }
    cmd.stdin(std::process::Stdio::null());
    cmd
}

fn run_script(tag: &str, body: &str) -> (String, String, String, Option<i32>) {
    let dir = scratch(tag);
    let script = dir.join("probe.sh");
    std::fs::write(&script, body).expect("write script");
    let output = base()
        .arg(&script)
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let result = (
        script.to_string_lossy().into_owned(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// One warning line for `var` naming `value`, with the first-lookup
/// errno suffix, at script line `line`.
fn warn(path: &str, line: u32, var: &str, value: &str) -> String {
    format!(
        "{path}: line {line}: warning: setlocale: {var}: cannot change locale ({value}): No such file or directory\n"
    )
}

#[test]
fn issue3097_element_assign_and_append_use_new_projection() {
    // Moved from the #3080 draft (assertions intact): element
    // assignment rebinds element 0; the funnel reads the NEW
    // projection. Append leaves element 0 alone: no warning at all.
    let (path, _stdout, stderr, code) =
        run_script("elem-assign", "LC_ALL=(C); LC_ALL[0]=bogus_XX; :\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, warn(&path, 1, "LC_ALL", "bogus_XX"));

    let (_path, _stdout, stderr, code) =
        run_script("append", "LC_ALL=(C); LC_ALL+=(bogus_XX); :\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
}

#[test]
fn issue3097_element_assign_line_attribution_and_repeat() {
    // Split across lines, the element bind's fire is attributed to
    // the assignment's own line.
    let (path, _stdout, stderr, code) =
        run_script("elem-lines", "LC_ALL=(C)\nLC_ALL[0]=bogus_XX\n:\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, warn(&path, 2, "LC_ALL", "bogus_XX"));

    // Two element binds in a row each fire once, with the value
    // bound at that moment; a different bogus name is a different
    // first lookup, so both carry the errno suffix.
    let (path, _stdout, stderr, code) = run_script(
        "elem-repeat",
        "LC_ALL=(C)\nLC_ALL[0]=bogus_XX\nLC_ALL[0]=bogus_YY\n:\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr,
        warn(&path, 2, "LC_ALL", "bogus_XX") + &warn(&path, 3, "LC_ALL", "bogus_YY")
    );
}

#[test]
fn issue3097_element_assign_projection_controls() {
    // The funnel reads the projection, not the assigned element:
    // writing a non-zero element leaves element 0 (C) in place, so
    // the fire — GNU runs it — succeeds silently. An implementation
    // that warns with the ASSIGNED value fails this cell.
    let (_path, _stdout, stderr, code) = run_script("elem1", "LC_ALL=(C); LC_ALL[1]=bogus_XX; :\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");

    // Restoring element 0 to a valid value fires silently too: the
    // only warning in the trail is the original compound bind.
    let (path, _stdout, stderr, code) =
        run_script("elem-restore", "LC_ALL=(bogus_XX); LC_ALL[0]=C; :\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, warn(&path, 1, "LC_ALL", "bogus_XX"));

    // Assoc: writing a non-"0" key leaves the "0" projection (C)
    // untouched — silent.
    let (_path, _stdout, stderr, code) = run_script(
        "assoc-other-key",
        "declare -A LC_ALL=([0]=C); LC_ALL[k]=bogus_XX; :\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
}

#[test]
fn issue3097_element_assign_assoc_zero_key() {
    // The assoc element channel fires with the "0"-key projection.
    let (path, _stdout, stderr, code) = run_script(
        "assoc-elem0",
        "declare -A LC_ALL=([0]=C); LC_ALL[0]=bogus_XX; :\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stderr, warn(&path, 1, "LC_ALL", "bogus_XX"));
}

#[test]
fn issue3097_for_and_export_channels_project() {
    // Moved from the #3080 draft (assertions intact): a for bind
    // stores a scalar into element 0 of the array; GNU warns once
    // with that value. Export's bind hook likewise.
    let (path, _stdout, stderr, code) =
        run_script("for", "LC_ALL=(C); for LC_ALL in bogus_XX; do :; done\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, warn(&path, 1, "LC_ALL", "bogus_XX"));

    let (path, _stdout, stderr, code) = run_script("export", "export LC_ALL=(bogus_XX); :\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, warn(&path, 1, "LC_ALL", "bogus_XX"));
}

#[test]
fn issue3097_for_chain_and_scalar_over_array_arms() {
    // Chain: two for binds over an array fire once each; the array
    // afterwards holds exactly the last bound element 0.
    let (path, stdout, stderr, code) = run_script(
        "for-chain",
        "LC_ALL=(C); for LC_ALL in bogus_XX bogus_YY; do :; done; echo \"elem0=[$LC_ALL] n=${#LC_ALL[@]}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "elem0=[bogus_YY] n=1\n");
    assert_eq!(
        stderr,
        warn(&path, 1, "LC_ALL", "bogus_XX") + &warn(&path, 1, "LC_ALL", "bogus_YY")
    );

    // The plain scalar bind over an existing array rides the same
    // store arm as the for bind (both land in element 0 through
    // apply_shell_assignment) and fires the same way.
    let (path, _stdout, stderr, code) =
        run_script("scalar-over-array", "LC_ALL=(C); LC_ALL=bogus_XX; :\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, warn(&path, 1, "LC_ALL", "bogus_XX"));

    // Assoc twins of both shapes: the bind lands on the "0" key.
    let (path, _stdout, stderr, code) = run_script(
        "for-assoc",
        "declare -A LC_ALL=([0]=C); for LC_ALL in bogus_XX; do :; done\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stderr, warn(&path, 1, "LC_ALL", "bogus_XX"));

    let (path, _stdout, stderr, code) = run_script(
        "scalar-over-assoc",
        "declare -A LC_ALL=([0]=C); LC_ALL=bogus_XX; :\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stderr, warn(&path, 1, "LC_ALL", "bogus_XX"));
}

#[test]
fn issue3097_declare_compound_names_projection_never_storage() {
    // Moved from the #3080 draft (assertions intact): value face
    // only (fire count is the #3066 declare family — see the module
    // header): every warning declare emits for a compound bind
    // names the projection; none may carry the storage render.
    let (_path, _stdout, stderr, code) =
        run_script("declare-compound", "declare LC_ALL=(bogus_XX); :\n");
    assert_eq!(code, Some(0));
    assert!(
        stderr.contains("cannot change locale (bogus_XX)"),
        "expected a warning naming the projection, got: {stderr}"
    );
    assert!(
        !stderr.contains('\u{1d}') && !stderr.contains("[0]="),
        "warning leaked the storage render: {stderr}"
    );
}

#[test]
fn issue3097_declare_assoc_names_projection_never_storage() {
    // Assoc compound through declare: the warning names the "0"
    // key's value, never the `(([0]=...))` storage render.
    let (_path, _stdout, stderr, code) =
        run_script("declare-assoc", "declare -A LC_ALL=([0]=bogus_XX); :\n");
    assert_eq!(code, Some(0));
    assert!(
        stderr.contains("cannot change locale (bogus_XX)"),
        "expected a warning naming the projection, got: {stderr}"
    );
    assert!(
        !stderr.contains('\u{1d}') && !stderr.contains("[0]="),
        "warning leaked the storage render: {stderr}"
    );

    // No "0" key: the projection is NULL -> "" -> the silent reset
    // path. GNU emits nothing at all.
    let (_path, _stdout, stderr, code) = run_script(
        "declare-assoc-nokey",
        "declare -A LC_ALL=([k]=bogus_XX); :\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");

    // The local twin of the declare channel names the projection
    // the same way.
    let (_path, _stdout, stderr, code) =
        run_script("local-compound", "f() { local LC_ALL=(bogus_XX); :; }; f\n");
    assert_eq!(code, Some(0));
    assert!(
        stderr.contains("cannot change locale (bogus_XX)"),
        "expected a warning naming the projection, got: {stderr}"
    );
    assert!(
        !stderr.contains('\u{1d}') && !stderr.contains("[0]="),
        "warning leaked the storage render: {stderr}"
    );

    // Storage itself is untouched by the funnel's reading: the
    // compound binding still reads back in stored form.
    let (_path, stdout, stderr, code) = run_script(
        "declare-readback",
        "declare LC_ALL=(bogus_XX); declare -p LC_ALL\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -a LC_ALL=([0]=\"bogus_XX\")\n");
    assert!(
        !stderr.contains('\u{1d}'),
        "warning leaked the storage render: {stderr}"
    );
}
