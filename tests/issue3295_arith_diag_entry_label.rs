//! Regression pins for rubash#3295 — diagnostics produced while
//! expanding inside arithmetic text are mislabeled with the `((:`
//! entry tag where GNU emits them untagged.
//!
//! GNU's rule (verified live against bash 5.3.20; scripts and
//! captures in ~/workspace/review-3288/probes/):
//!   - `execute_arith_command` sets `this_command_name = "(("`
//!     (execute_cmd.c:3903 before expansion, :3931 before evalexp),
//!     but the nested windows NULL it while they run:
//!     `arithsub` (subst.c:10871-10874) and `array_expand_index`
//!     (arrayfunc.c:1356-1374). A diagnostic produced inside one of
//!     those windows therefore carries NO label; a diagnostic
//!     produced by the `(( ))` command's own evalexp carries `((:`.
//!   - S4 `(( x=$(( ref=1 )) ))` / `(( y=$(( ref=0, LC_ALL=5 )) ))`:
//!     both diags untagged (nested `$(( ))` window), diag before
//!     the setlocale warning on line 3.
//!   - S6 `(( x=${a[ref=1]} ))`: the diag untagged (subscript
//!     window).
//!   - S7 `(( ref=1, x=${a[ref2=1]} ))`: FIRST diag untagged
//!     (subscript window, expanded before evalexp), SECOND diag
//!     tagged `((:` (the command's own evalexp bind).
//!   - S1/S2/S5 are byte-identical with GNU on head and must stay
//!     so (regression): S1's comsub-subscript diag is untagged and
//!     its outer bind diag tagged; S2's single outer bind diag is
//!     tagged; S5's two diags are both untagged.
//!
//! Root cause (rubash): the single shared evaluation-order event
//! log (`ARITH_EVAL_EVENTS`) carries no per-event label, and the
//! `(( ))` entry drains it once at its boundary with `Some("(( ")`.
//! Nested `$(( ))` / subscript evaluations inside the entry's
//! pre-expansion run through raw-eval helpers that never drain
//! (`eval_conditional_arith_value_categorized_with_writes`,
//! `eval_indexed_subscript_deferred`), so their diagnostics sit in
//! the log until the outer boundary stamps them `((:`.
//!
//! All expectations are the literal GNU bash 5.3.20 outputs for the
//! same script files, re-verified 2026-11-13 against the vendored
//! 5.3.20 binary (~/workspace/rubash-audit/build-gnu/bash-5.3/bash).
#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3295-{tag}-{}", std::process::id()));
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

#[test]
fn issue3295_s4_nested_expansion_untagged() {
    let (path, stdout, stderr, code) = run_script(
        "s4",
        "declare -n ref\n(( x=$(( ref=1 )) )); echo \"rc=$? x=[$x]\"\n(( y=$(( ref=0, LC_ALL=5 )) )); echo \"rc=$? y=[$y]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0 x=[1]\nrc=0 y=[5]\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 2: `1': not a valid identifier\n\
             {path}: line 3: `0': not a valid identifier\n\
             {path}: line 3: warning: setlocale: LC_ALL: cannot change locale (5): No such file or directory\n"
        )
    );
}

#[test]
fn issue3295_s6_subscript_in_arith_untagged() {
    let (path, stdout, stderr, code) = run_script(
        "s6",
        "declare -n ref\na=(x y z)\n(( x=${a[ref=1]} )); echo \"rc=$? x=[$x]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1 x=[0]\n");
    assert_eq!(
        stderr,
        format!("{path}: line 3: `1': not a valid identifier\n")
    );
}

#[test]
fn issue3295_s7_first_untagged_second_tagged() {
    let (path, stdout, stderr, code) = run_script(
        "s7",
        "declare -n ref\ndeclare -n ref2\na=(x y z)\n(( ref=1, x=${a[ref2=1]} )); echo \"rc=$? x=[$x]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1 x=[0]\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 4: `1': not a valid identifier\n\
             {path}: line 4: ((: `1': not a valid identifier\n"
        )
    );
}

#[test]
fn issue3295_s1_comsub_subscript_regression() {
    let (path, stdout, stderr, code) = run_script(
        "s1",
        "declare -n ref\ndeclare -n ref2\na=(x y z)\n(( ref=1, x=$(echo ${a[ref2=1]}) )); echo \"rc=$? x=[$x]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1 x=[0]\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 4: `1': not a valid identifier\n\
             {path}: line 4: ((: `1': not a valid identifier\n"
        )
    );
}

#[test]
fn issue3295_s2_assign_comsub_regression() {
    let (path, stdout, stderr, code) = run_script(
        "s2",
        "declare -n ref\n(( ref=1, x=${y:=$(echo hi)} )); echo \"rc=$? x=[$x] y=[$y]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1 x=[0] y=[hi]\n");
    assert_eq!(
        stderr,
        format!("{path}: line 2: ((: `1': not a valid identifier\n")
    );
}

#[test]
fn issue3295_s5_word_then_fatal_regression() {
    let (path, stdout, stderr, code) = run_script(
        "s5",
        "declare -n ref\na=(x y z)\necho \"${a[ref=1]} $(( ref=1, 1/0 ))\"; echo \"rc=$?\"\n",
    );
    // The fatal expansion discards the rest of line 3 (both echos),
    // so stdout is empty and the script exits 1.
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 3: `1': not a valid identifier\n\
             {path}: line 3: `1': not a valid identifier\n\
             {path}: line 3: ref=1, 1/0 : division by 0 (error token is \"0 \")\n"
        )
    );
}
