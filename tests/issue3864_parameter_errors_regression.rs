//! rubash#3864 — the #3855 bad-substitution pre-scan gate
//! (`parameter_errors.rs`, the `braced_name_is_bad_substitution(inner)`
//! check added for #3502) regressed 30+ pin cells: it fires on spans
//! whose verdict the real expansion owns, and reports them with the
//! scan's whole-word display and status-1 class.
//!
//! - issue3500 compound elements: the scan names the raw assignment
//!   value (`([0]=${-x})`) where GNU names the element word
//!   (`\[0\]=${-x}`, arrayfunc.c:557 `expand_compound_array_assignment`).
//! - issue2650 `$'name'` extquote names: the gate judges the raw
//!   `$'x1'` literal where the real expansion decodes it to the name
//!   `x1` in a quoted context (subst.c:7583 judges the extracted
//!   NAME, not the quoted spelling).
//! - audit B4-F08 POSIX fatal class: the scan's status 1 downgrades
//!   the subst.c:10285-10288 fatal arm (POSIX + non-interactive +
//!   compat > 43) to a plain DISCARD.
//! - golden funsub-valsub / #3502 guards: the funsub face (#3869) and
//!   the #3502 inner-only report must stay green through the repair.
//!
//! Fail-first note: this file was written BEFORE the repair, against
//! the analyzed pre-fix behavior (the build chain is down — X299
//! tunnel refused, box wallet empty, local build banned — so no test
//! in this file was executed in this lane; the captain re-runs the
//! head census after the chain recovers). On the pre-fix tree the
//! regression nails fail by construction: the gate fires on each of
//! their spans and the scan's verdict pre-empts the real expansion.
//! The guards (#3502 inner-only, pin-7 ordering, compat43, valid
//! compound) pass on both trees.

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3864-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

/// Run `script` (one line) under rubash as a script file; return
/// (stdout, stderr, rc).
fn run_script(script: &str) -> (String, String, i32) {
    let dir = tempdir("case");
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let rc = output.status.code().unwrap_or(-1);
    let _ = std::fs::remove_dir_all(&dir);
    (stdout, stderr, rc)
}

fn rubash_c(script: &str) -> (String, String, Option<i32>) {
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

/// Extract the word a `bad substitution` diagnostic names: the stderr
/// line is `<argv0>: line 1: <word>: bad substitution`.
fn diagnostic_word(stderr: &str) -> Option<String> {
    let line = stderr.lines().find(|l| l.contains("bad substitution"))?;
    let after = line.split_once(": line 1: ")?.1;
    after.strip_suffix(": bad substitution").map(str::to_string)
}

/// The diagnostic must name exactly `word`, be the only diagnostic
/// line, and the script must exit `rc`.
fn check_bad_substitution(script: &str, word: &str, rc: i32) {
    let (_stdout, stderr, got_rc) = run_script(script);
    assert_eq!(
        got_rc, rc,
        "script {script:?}: rc {got_rc}, stderr: {stderr}"
    );
    assert_eq!(
        stderr
            .lines()
            .filter(|l| l.contains("bad substitution"))
            .count(),
        1,
        "script {script:?}: expected exactly one diagnostic, stderr: {stderr}"
    );
    assert_eq!(
        diagnostic_word(&stderr).as_deref(),
        Some(word),
        "script {script:?}: stderr: {stderr}"
    );
}

// --- issue3500: compound element diagnostic words --------------------

#[test]
fn compound_indexed_element_names_element_word() {
    check_bad_substitution("a=([0]=${-x})\n", "\\[0\\]=${-x}", 1);
}

#[test]
fn compound_assoc_element_names_bare_value() {
    check_bad_substitution("declare -A h=([k]=${-x})\n", "${-x}", 1);
}

#[test]
fn compound_nested_bad_substitution_names_inner_fragment() {
    // A bad substitution nested inside a `:-` word of an element names
    // the inner fragment, not the element and not the whole value.
    check_bad_substitution("a=([0]=${x:-${-y}})\n", "${-y}", 1);
}

#[test]
fn compound_valid_element_stays_quiet() {
    // Guard: skipping the scan gate for compound values must not
    // silence (or break) valid elements.
    let (stdout, stderr, rc) = run_script("a=([0]=${v:-d}); echo ${a[0]}\n");
    assert_eq!(rc, 0, "stderr: {stderr}");
    assert!(!stderr.contains("bad substitution"), "stderr: {stderr}");
    assert_eq!(stdout, "d\n");
}

// --- issue2650: `$'name'` extquote names ------------------------------

#[test]
fn extquote_name_top_level_pattern_expands() {
    // GNU expands `${$'x1'%$'t'}` to the `x1` value with the `%$'t'`
    // pattern stripped; the scan gate must not call the `$'x1'`
    // spelling a bad substitution.
    let (stdout, stderr, code) = rubash_c("x=notOK; x1=not\necho \"[${x#${$'x1'%$'t'}}]\"\n");
    assert_eq!(stdout, "[tOK]\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    let (stdout, stderr, code) = rubash_c("x1=not\necho \"[${$'x1'%$'t'}]\"\n");
    assert_eq!(stdout, "[no]\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn extquote_name_bare_expands_in_quotes() {
    // A bare `$'name'` as the whole braced name decodes in a quoted
    // context (local GNU 5.2.21 probe: `[hello]`); the gate must judge
    // the decoded name, not the literal.
    let (stdout, stderr, code) = rubash_c("x1=hello\necho \"[${$'x1'}]\"\n");
    assert_eq!(stdout, "[hello]\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

// --- audit B4-F08: POSIX bad-substitution fatal class -----------------

#[test]
fn posix_bad_substitution_default_stays_fatal() {
    // POSIX mode, default compat: a bad substitution aborts the
    // script (subst.c:10285-10288 fatal arm); `ALIVE` never prints.
    let (stdout, _stderr, rc) = run_script("set -o posix\necho ${}\necho ALIVE\n");
    assert_eq!(stdout, "", "stdout: {stdout}");
    assert_eq!(rc, 1);
}

#[test]
fn posix_bad_substitution_compat43_stays_nonfatal() {
    // Guard: at compat <= 43 the same shape is the plain error class
    // (command fails with 1, the script continues).
    let (stdout, _stderr, rc) =
        run_script("set -o posix\nshopt -s compat43\necho ${}\necho ALIVE rc=$?\n");
    assert_eq!(stdout, "ALIVE rc=1\n", "stdout: {stdout}");
    assert_eq!(rc, 0);
}

// --- golden funsub-valsub shapes (funsub face, #3869) -----------------

#[test]
fn funsub_valsub_assignment_shapes() {
    // Golden fixture funsub-valsub f1/f11: a funsub body is command
    // text, never a parameter name — the scan must not judge it.
    let (stdout, stderr, code) = rubash_c("v=${ echo 4; }; echo \"[$v] rc=$?\"\n");
    assert_eq!(stdout, "[4] rc=0\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    let (stdout, stderr, code) = rubash_c("unset sv; v=${ sv=9; echo x; }; echo \"sv=$sv\"\n");
    assert_eq!(stdout, "sv=9\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

// --- #3502 guards: the repair must not reopen #3502 -------------------

#[test]
fn question_error_bad_substitution_word_still_inner_only() {
    let (stdout, stderr, code) = rubash_c("unset v; echo \"${v:?${-x}}\"\n");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("${-x}: bad substitution"),
        "stderr: {stderr}"
    );
    assert!(
        !stderr.contains("v: "),
        "outer :? report must stay suppressed, stderr: {stderr}"
    );
    assert_eq!(code, Some(1));
}

#[test]
fn bad_substitution_before_question_error_still_wins() {
    // GNU expands left-to-right: `${-y}` errors before `${a:?x}` is
    // reached, so the bad substitution — not `a: x` — is reported.
    let (stdout, stderr, _) = rubash_c("unset a; echo \"${-y}${a:?x}\"\n");
    assert_eq!(stdout, "");
    assert!(stderr.contains("bad substitution"), "stderr: {stderr}");
    assert!(!stderr.contains("a: x"), "stderr: {stderr}");
}
