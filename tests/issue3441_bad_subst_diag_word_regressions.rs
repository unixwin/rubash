//! rubash#3441 — `bad substitution` must name the whole word being expanded,
//! not the `${...}` fragment the assignment split re-rooted the walk at.
//!
//! GNU anchor: subst.c:10278 `parameter_brace_expand`'s `bad substitution`
//! default reports `string` — the whole word `expand_word_internal` was
//! expanding (call site subst.c:10800). `expand_word_internal` never splits
//! an assignment-shaped argument word at `=` for parameter expansion (the
//! `W_ASSIGNMENT` flag only steers tilde handling, subst.c:11432); only a
//! genuine assignment STATEMENT expands its RHS as its own word
//! (subst.c:4357 `expand_string_assignment`).
//!
//! Rubash's `expand_word_mut_with_context` split `a=${-x}` at `=` and
//! expanded the RHS `${-x}` alone, so the diagnostic named the fragment.
//! The fix records the whole word as the pending diagnostic word across
//! the split (install-if-absent, so nested operator words keep their own).
//!
//! Expectations verified against GNU bash 5.3.20 (script files, local
//! probes probe3441*.sh). Each case runs as a one-line script so the
//! diagnostic is `rubash: line 1: <word>: bad substitution`; the helper
//! extracts `<word>` exactly (a `contains` check cannot distinguish
//! `a=${-x}` from `${-x}` — the latter is a suffix of the former).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3441-{tag}-{}", std::process::id()));
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

// --- The issue's form: assignment-shaped argument words -------------

#[test]
fn issue3441_unquoted_assignment_word_names_whole_word() {
    check_bad_substitution("echo a=${-x}\n", "a=${-x}", 1);
}

#[test]
fn issue3441_prefix_text_before_fragment_kept() {
    check_bad_substitution("echo a=b${-x}\n", "a=b${-x}", 1);
}

#[test]
fn issue3441_longer_name_before_equals_kept() {
    check_bad_substitution("echo ab=${-x}\n", "ab=${-x}", 1);
}

#[test]
fn issue3441_trailing_text_after_fragment_kept() {
    check_bad_substitution("echo a=${-x}b\n", "a=${-x}b", 1);
}

#[test]
fn issue3441_subscript_gate_names_whole_word() {
    check_bad_substitution("echo a=${x[0]y}\n", "a=${x[0]y}", 1);
}

#[test]
fn issue3441_plain_dollar_before_fragment_kept() {
    check_bad_substitution("echo a=$x${-y}\n", "a=$x${-y}", 1);
}

#[test]
fn issue3441_plain_dollar_after_fragment_kept() {
    check_bad_substitution("echo a=${-x}$y\n", "a=${-x}$y", 1);
}

#[test]
fn issue3441_second_equals_stays_inside_word() {
    check_bad_substitution("echo a=b=c=${-x}\n", "a=b=c=${-x}", 1);
}

#[test]
fn issue3441_append_form_names_whole_word() {
    check_bad_substitution("echo a+=${-x}\n", "a+=${-x}", 1);
}

#[test]
fn issue3441_transform_tail_names_whole_word() {
    check_bad_substitution("echo a=${-x@Q}\n", "a=${-x@Q}", 1);
}

#[test]
fn issue3441_pattern_tail_names_whole_word() {
    check_bad_substitution("echo a=${-x%y}\n", "a=${-x%y}", 1);
}

#[test]
fn issue3441_substitution_tail_names_whole_word() {
    check_bad_substitution("echo a=${-x/y/z}\n", "a=${-x/y/z}", 1);
}

#[test]
fn issue3441_default_operator_tail_names_whole_word() {
    check_bad_substitution("echo a=${-x:-d}e\n", "a=${-x:-d}e", 1);
}

#[test]
fn issue3441_later_fragment_names_whole_word() {
    check_bad_substitution("echo a=${v:1:1}${-x}\n", "a=${v:1:1}${-x}", 1);
}

// --- Other positions that route through the same split -------------

#[test]
fn issue3441_builtin_argument_names_whole_word() {
    check_bad_substitution("declare a=${-x}\n", "a=${-x}", 1);
}

#[test]
fn issue3441_printf_argument_names_whole_word() {
    check_bad_substitution("printf '%s\\n' a=${-x}\n", "a=${-x}", 1);
}

#[test]
fn issue3441_double_bracket_operand_names_whole_word() {
    check_bad_substitution("[[ a=${-x} == x ]]\n", "a=${-x}", 1);
}

#[test]
fn issue3441_for_list_word_names_whole_word() {
    check_bad_substitution("for w in a=${-x}; do echo \"$w\"; done\n", "a=${-x}", 1);
}

#[test]
fn issue3441_case_subject_names_whole_word() {
    check_bad_substitution("case a=${-x} in *) echo hi;; esac\n", "a=${-x}", 1);
}

// --- Guard cases: already correct, must not regress ----------------

#[test]
fn issue3441_word_is_fragment_reports_fragment() {
    check_bad_substitution("echo ${-x}\n", "${-x}", 1);
}

#[test]
fn issue3441_no_equals_word_reports_whole_word() {
    check_bad_substitution("echo pre${-x}post\n", "pre${-x}post", 1);
}

#[test]
fn issue3441_unclosed_subscript_prescan_class_unchanged() {
    // Caught by the whole-word pre-scan (malformed_braced_reference),
    // never by the split path.
    check_bad_substitution("echo a=${x[}\n", "a=${x[}", 1);
}

#[test]
fn issue3441_operator_word_keeps_its_own_word() {
    // The `:-` word is expanded as its own word at that level (GNU
    // subst.c:7993 expand_string_for_rhs), so it — not the outer word —
    // is the diagnostic word.
    check_bad_substitution("echo ${x:-a=${-y}}\n", "a=${-y}", 1);
}

#[test]
fn issue3441_nested_fragment_keeps_its_own_word() {
    // The outer fragment's gate consumes the pending record before the
    // nested operator word expands, so the nested bad fragment still
    // names itself, not the enclosing word.
    check_bad_substitution("echo a=${x:-${-y}}\n", "${-y}", 1);
}

#[test]
fn issue3441_genuine_assignment_statement_unchanged() {
    // A real assignment statement expands its RHS as its own word
    // (subst.c:4357), so the RHS text is the diagnostic word.
    check_bad_substitution("x=a=${-x}\n", "a=${-x}", 1);
}

#[test]
fn issue3441_valid_special_parameter_stays_quiet() {
    let (stdout, stderr, rc) = run_script("echo a=${-}\necho ok\n");
    assert_eq!(rc, 0, "stderr: {stderr}");
    assert!(stderr.is_empty(), "stderr: {stderr}");
    assert!(stdout.contains("ok\n"), "stdout: {stdout}");
}
