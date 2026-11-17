//! rubash#3500 — `bad substitution` inside a compound assignment must
//! name the ELEMENT word GNU was expanding, not the `${...}` fragment.
//!
//! GNU anchor: arrayfunc.c:557 `expand_compound_array_assignment`
//! tokenizes the raw parenthesized body first (parse_string_to_word_list,
//! arrayfunc.c:581) and expands each element word individually
//! (expand_words_no_vars, arrayfunc.c:610); `parameter_brace_expand`'s
//! `bad substitution` default reports `string` — the word being expanded
//! (subst.c:10278). For an indexed `[sub]=value` element,
//! `quote_array_assignment_chars` (arrayfunc.c:606, :1108-1129) runs
//! `quote_assign` (arrayfunc.c:938-977) over the word first, so the
//! diagnostic cites the subscript span backslash-quoted (`\[0\]=${-x}`).
//! Associative elements are deferred (arrayfunc.c:594-599) and expanded
//! as key/value separately (assign_assoc_from_kvlist, arrayfunc.c:630),
//! so their diagnostic names the bare value (`${-x}`); a quoted element
//! is one quoted word whose text is the fragment itself. A nested
//! fragment keeps its own word (same rule as rubash#3441).
//!
//! Expectations verified against GNU bash 5.3.20 (script files, local
//! probes probe3500_gnu.sh). Each case runs as a one-line script so the
//! diagnostic is `rubash: line 1: <word>: bad substitution`; the helper
//! extracts `<word>` exactly (a `contains` check cannot distinguish
//! `[0]=${-x}` from `${-x}` — the latter is a suffix of the former).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3500-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

/// Run `script` (one line) under rubash and return (stdout, stderr, rc).
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

/// Extract the word a `bad substitution` diagnostic names: the stderr line
/// is `<argv0>: line 1: <word>: bad substitution`.
fn diagnostic_word(stderr: &str) -> Option<String> {
    let line = stderr.lines().find(|l| l.contains("bad substitution"))?;
    let after = line.split_once(": line 1: ")?.1;
    after.strip_suffix(": bad substitution").map(str::to_string)
}

/// The diagnostic must name exactly `word`, be the only diagnostic line,
/// and the script must exit `rc`.
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

/// Like `check_bad_substitution`, but tolerates extra diagnostic lines:
/// GNU reports only the first bad element of a multi-bad compound word,
/// while rubash may still report each (multi-report, rubash#3441 §6.1 /
/// review O1 — a separate mechanism). Only the FIRST diagnostic's word
/// and the exit code are asserted here.
fn check_first_bad_substitution(script: &str, word: &str, rc: i32) {
    let (_stdout, stderr, got_rc) = run_script(script);
    assert_eq!(
        got_rc, rc,
        "script {script:?}: rc {got_rc}, stderr: {stderr}"
    );
    assert_eq!(
        diagnostic_word(&stderr).as_deref(),
        Some(word),
        "script {script:?}: stderr: {stderr}"
    );
}

/// A valid compound assignment stays quiet and stores the element.
fn check_compound_ok(script: &str, stdout: &str) {
    let (got_stdout, stderr, got_rc) = run_script(script);
    assert_eq!(
        got_rc, 0,
        "script {script:?}: rc {got_rc}, stderr: {stderr}"
    );
    assert!(
        !stderr.contains("bad substitution"),
        "script {script:?}: unexpected diagnostic, stderr: {stderr}"
    );
    assert_eq!(got_stdout, stdout, "script {script:?}: stdout mismatch");
}

// --- Repair forms: indexed element words (GNU cites the element) ------

#[test]
fn issue3500_indexed_element_names_element_word() {
    check_bad_substitution("a=([0]=${-x})\n", "\\[0\\]=${-x}", 1);
}

#[test]
fn issue3500_plain_element_names_whole_element() {
    check_bad_substitution("a=(x${-y})\n", "x${-y}", 1);
}

#[test]
fn issue3500_glued_element_names_whole_element() {
    check_bad_substitution("a=(pre${-x}post)\n", "pre${-x}post", 1);
}

#[test]
fn issue3500_second_element_names_its_own_word() {
    check_bad_substitution("a=([0]=ok [1]=${-x})\n", "\\[1\\]=${-x}", 1);
}

#[test]
fn issue3500_element_trailing_text_kept() {
    check_bad_substitution("a=([0]=${-x}foo)\n", "\\[0\\]=${-x}foo", 1);
}

#[test]
fn issue3500_two_digit_subscript_kept() {
    check_bad_substitution("a=([10]=${-x})\n", "\\[10\\]=${-x}", 1);
}

#[test]
fn issue3500_later_element_after_literal() {
    check_bad_substitution("a=(foo [0]=${-x})\n", "\\[0\\]=${-x}", 1);
}

#[test]
fn issue3500_append_compound_names_element_word() {
    check_bad_substitution("a+=([0]=${-x})\n", "\\[0\\]=${-x}", 1);
}

#[test]
fn issue3500_declare_indexed_names_element_word() {
    check_bad_substitution("declare -a a=([0]=${-x})\n", "\\[0\\]=${-x}", 1);
}

#[test]
fn issue3500_first_bad_element_wins() {
    check_first_bad_substitution("a=([0]=${-x} [1]=${-y})\n", "\\[0\\]=${-x}", 1);
}

#[test]
fn issue3500_split_subscript_element_names_value_word() {
    // `[0]` and `=${-x}` are separate words; GNU cites the bad one.
    check_bad_substitution("a=([0] =${-x})\n", "=${-x}", 1);
}

// --- Guard forms: GNU cites the fragment (must stay) -------------------

#[test]
fn issue3500_quoted_element_keeps_fragment_word() {
    check_bad_substitution("a=([0]=\"${-x}\")\n", "${-x}", 1);
}

#[test]
fn issue3500_quoted_bare_element_keeps_fragment_word() {
    check_bad_substitution("a=( \"${-x}\" )\n", "${-x}", 1);
}

#[test]
fn issue3500_assoc_element_keeps_value_word() {
    check_bad_substitution("declare -A h=([k]=${-x})\n", "${-x}", 1);
}

#[test]
fn issue3500_assoc_kvpair_keeps_value_word() {
    check_bad_substitution("declare -A h=(k ${-x})\n", "${-x}", 1);
}

#[test]
fn issue3500_nested_fragment_keeps_its_own_word() {
    check_bad_substitution("a=([0]=${x:-${-y}})\n", "${-y}", 1);
}

#[test]
fn issue3500_bare_fragment_element_keeps_fragment_word() {
    check_bad_substitution("a=(x y ${-z})\n", "${-z}", 1);
}

// --- Guard forms: valid compounds stay quiet ---------------------------

#[test]
fn issue3500_valid_compound_stays_quiet() {
    check_compound_ok("a=([0]=ok); echo ${a[0]}\n", "ok\n");
}

#[test]
fn issue3500_valid_single_quoted_element_stays_quiet() {
    check_compound_ok("a=([0]='${-x}'); echo ${a[0]}\n", "${-x}\n");
}
