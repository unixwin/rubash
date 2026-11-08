//! Regression pins for rubash#3065 — the shared sv_locale funnel
//! (`locale_assignment_warning`) read the RAW compound storage string
//! of an array-bound locale variable instead of its scalar
//! projection:
//!   LC_ALL=(C); :
//! stayed silent in GNU but warned in rubash with the storage render
//! embedded in the message:
//!   warning: setlocale: LC_ALL: cannot change locale (\x1d([0]="C")): ...
//!
//! GNU ownership: sv_locale (variables.c:6272) reads the binding via
//! get_string_value (variables.c:2495) -> get_variable_value
//! (variables.c:2473): an indexed array projects to element 0, an
//! assoc to its "0" key (NULL when absent), anything else to the
//! stored value itself. set_locale_var (locale.c:185) then attempts
//! setlocale with that scalar and renders THAT value in the warning
//! (locale.c:226 / locale.c:302, `: <strerror>` only when errno is
//! set — glibc leaves errno set only on the FIRST failed lookup of a
//! given name per process). Category precedence reads go through
//! get_locale_var (locale.c:343), which reads bindings the same way.
//! A locale array with no element 0 (or an assoc with no "0" key)
//! projects to NULL, which set_locale_var treats as "" — the silent
//! reset path.
//!
//! The declare-compound fire COUNT is deliberately not pinned here:
//! GNU fires twice for `declare LC_ALL=(...)` (the array bind's
//! internal hack plus declare's own per-variable
//! stupidly_hack_special_variables call, builtins/declare.def:1105),
//! while rubash's declare channel fires once — that count gap is the
//! declare-channel family tracked by #3066 (same missing declare-side
//! hack call as the declare conversion bind).
//!
//! Scope split (captain review of PR #3080, 2026-11-08): three cells
//! originally drafted here exposed sibling gaps with different
//! mechanisms and were moved to #3097 with their assertions intact —
//! the element-assign and for-bind channels never fire the funnel for
//! an array locale variable at all, and the declare channel's funnel
//! call projects through `attr_of` while declare maintains array-ness
//! in the mark sets, so its warnings still name the storage render
//! (the declare VALUE face lives in #3097 too; only the fire count
//! stays with #3066).
//!
//! All expectations are the literal GNU bash 5.3.20 outputs for the
//! same script files in a locale-clean environment (only C/POSIX
//! locales installed; the bogus names can never resolve).
#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3065-{tag}-{}", std::process::id()));
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
fn issue3065_valid_element0_is_silent() {
    // Element 0 is a valid locale: GNU attempts setlocale("C") and
    // succeeds — no warning. The base warned on the storage string.
    let (_path, stdout, stderr, code) = run_script("elem0-c", "LC_ALL=(C); :\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
}

#[test]
fn issue3065_warning_names_element0_only() {
    let (path, _stdout, stderr, code) = run_script("elem0-bogus", "LC_ALL=(bogus_XX); :\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, warn(&path, 1, "LC_ALL", "bogus_XX"));

    // Later elements are invisible to the funnel, in both orders.
    let (path, _stdout, stderr, code) =
        run_script("multi", "LC_ALL=(bogus_XX C); :\nLC_ALL=(C bogus_XX); :\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, warn(&path, 1, "LC_ALL", "bogus_XX"));

    // An element containing a space projects whole.
    let (path, _stdout, stderr, code) = run_script("space", "LC_ALL=(\"bogus XX\"); :\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, warn(&path, 1, "LC_ALL", "bogus XX"));
}

#[test]
fn issue3065_no_element0_projects_to_empty() {
    // No element 0 -> get_string_value is NULL -> the silent reset
    // path, exactly like an empty LC_ALL.
    let (_path, _stdout, stderr, code) = run_script(
        "no-elem0",
        "LC_ALL=([2]=bogus_XX); :\nLC_ALL=(\"\" bogus_XX); :\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
}

#[test]
fn issue3065_category_array_projects() {
    let (path, _stdout, stderr, code) = run_script("ctype", "LC_CTYPE=(bogus_XX); :\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, warn(&path, 1, "LC_CTYPE", "bogus_XX"));
}

#[test]
fn issue3065_category_suppressed_by_projected_lc_all() {
    // LC_ALL's projection is non-empty, so the category bind is
    // suppressed (locale.c: the category arm only runs while LC_ALL
    // is empty) — only the LC_ALL bind itself warns.
    let (path, _stdout, stderr, code) =
        run_script("suppress", "LC_ALL=(bogus_XX); LC_CTYPE=(bogus_YY); :\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, warn(&path, 1, "LC_ALL", "bogus_XX"));
}

#[test]
fn issue3065_category_reads_projected_lang() {
    // get_locale_var precedence reaches LANG; its projection (element
    // 0) is the value the LC_CTYPE attempt uses and names.
    let (path, _stdout, stderr, code) = run_script("via-lang", "LANG=(bogus_XX); LC_CTYPE=; :\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, warn(&path, 1, "LC_CTYPE", "bogus_XX"));
}

#[test]
fn issue3065_assoc_projects_zero_key() {
    // Plain assignment onto an assoc-marked name: the projection is
    // the "0" key, absent key -> NULL -> silent.
    let (path, _stdout, stderr, code) = run_script(
        "assoc",
        "declare -A LC_ALL; LC_ALL=([0]=bogus_XX); :\nLC_ALL=([k]=bogus_XX); :\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stderr, warn(&path, 1, "LC_ALL", "bogus_XX"));
}

#[test]
fn issue3065_arith_category_gate_reads_projected_lc_all() {
    // The arithmetic channel's probe snapshot of LC_ALL projects too:
    // element 0 "" leaves the category gate open, so the LC_CTYPE
    // bind warns; a bogus element 0 closes it.
    let (path, _stdout, stderr, code) =
        run_script("arith-gate", "LC_ALL=(\"\" x); ((LC_CTYPE = 5)); :\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, warn(&path, 1, "LC_CTYPE", "5"));

    let (path, _stdout, stderr, code) =
        run_script("arith-suppress", "LC_ALL=(bogus_XX); ((LC_CTYPE = 5)); :\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, warn(&path, 1, "LC_ALL", "bogus_XX"));
}

#[test]
fn issue3065_storage_and_readback_untouched() {
    // The fix is in the funnel's reading, not in storage: declare -p
    // and the scalar read keep their established forms, and a later
    // scalar bind replaces the array with a single scalar warning
    // trail (only the array bind warns).
    let (path, stdout, stderr, code) = run_script(
        "readback",
        "LC_ALL=(bogus_XX); declare -p LC_ALL; echo \"scalar=[$LC_ALL]\"; LC_ALL=C; :\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "declare -a LC_ALL=([0]=\"bogus_XX\")\nscalar=[bogus_XX]\n"
    );
    assert_eq!(stderr, warn(&path, 1, "LC_ALL", "bogus_XX"));
}
