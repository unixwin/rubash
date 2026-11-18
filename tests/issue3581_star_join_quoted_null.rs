//! rubash#3581 (narrowed) — a `bad substitution` inside a pattern word must
//! be diagnosed for the QUOTED `[*]` / `*` join forms holding exactly one
//! `""` element, exactly as it already is for `[@]` / `@`.
//!
//! GNU anchors (bash 5.3, subst.c):
//! - `#`/`%`: `parameter_brace_expand`'s `case '#': case '%':`
//!   (subst.c:10320-10327) breaks out before
//!   `parameter_brace_remove_pattern` when `value == 0 || *value == '\0'`
//!   — the pattern is never expanded, so a bad substitution inside it is
//!   never diagnosed. The value word is the parameter's expanded word:
//!   for a quoted one-`""` list it carries the QUOTED_NULL marker
//!   (subst.c:10133-10140, `QUOTED_NULL(temp)` under `Q_DOUBLE_QUOTES`),
//!   so `*temp == '\0'` does NOT fire and the pattern IS expanded.
//! - `/`: `parameter_brace_patsub` (subst.c:9362-9367) and `^`/`,`/`~`:
//!   `parameter_brace_casemod` (subst.c:9604-9608) only test
//!   `value == 0`, so a one-`""` list is present and its pattern IS
//!   expanded.
//! - Operators apply PER ELEMENT for `[*]` / `*` exactly as for `[@]` /
//!   `@` (oracle-verified: `a=(xx hyy); "${a[*]#h}"` is `xx yy`, not
//!   `xx hyy`); the elements are then joined with IFS[0]
//!   (subst.c:2902 `string_list_dollar_star`).
//!
//! Root cause (this lane): the `AtModified` segment classifier
//! (`at_modified_segment_body`, command_substitution_values.rs) admitted
//! only `@` / `name[@]` targets, so a `"${a[*]#...}"` / `"${*#...}"`
//! segment inside a mixed word fell through to the scalar braced path,
//! whose removal gate hardcodes `empty_is_silent = true`
//! (`expand_parameter_pattern_removal`) and never sees the quoted flag.
//! The `[@]` / `@` segments already route through the quoted-aware list
//! helpers; `[*]` / `*` now do the same.
//!
//! Expectations verified against GNU bash 5.3.20 (local oracle probes).
//! Each case runs as a one-line script so the diagnostic is
//! `rubash: line 1: <word>: bad substitution`. Silent cases assert the
//! exact stdout, rc 0 and empty stderr; error cases assert rc 1,
//! exactly one diagnostic, and the exact word it names.

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3581-{tag}-{}", std::process::id()));
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

/// The script must be silent: exact stdout, rc 0, empty stderr.
fn check_silent(script: &str, expected_stdout: &str) {
    let (stdout, stderr, rc) = run_script(script);
    assert_eq!(rc, 0, "script {script:?}: rc {rc}, stderr: {stderr}");
    assert!(
        stderr.is_empty(),
        "script {script:?}: expected empty stderr, got: {stderr}"
    );
    assert_eq!(
        stdout, expected_stdout,
        "script {script:?}: stdout mismatch"
    );
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

// --- Fail-first: quoted one-"" `[*]` / `*` removal must diagnose --------

#[test]
fn issue3581_array_star_prefix_removal_diagnosed_when_one_empty_quoted() {
    check_bad_substitution("a=(\"\"); echo X\"${a[*]#*a=${-y}}\"Y", "*a=${-y}", 1);
}

#[test]
fn issue3581_positional_star_prefix_removal_diagnosed_when_one_empty_quoted() {
    check_bad_substitution("set -- \"\"; echo X\"${*#*a=${-y}}\"Y", "*a=${-y}", 1);
}

#[test]
fn issue3581_array_star_suffix_removal_diagnosed_when_one_empty_quoted() {
    check_bad_substitution("a=(\"\"); echo X\"${a[*]%*a=${-y}}\"Y", "*a=${-y}", 1);
}

#[test]
fn issue3581_positional_star_suffix_removal_diagnosed_when_one_empty_quoted() {
    check_bad_substitution("set -- \"\"; echo X\"${*%*a=${-y}}\"Y", "*a=${-y}", 1);
}

#[test]
fn issue3581_array_star_prefix_removal_diagnosed_whole_word() {
    check_bad_substitution("a=(\"\"); echo \"${a[*]#*a=${-y}}\"", "*a=${-y}", 1);
}

#[test]
fn issue3581_positional_star_prefix_removal_diagnosed_whole_word() {
    check_bad_substitution("set -- \"\"; echo \"${*#*a=${-y}}\"", "*a=${-y}", 1);
}

// --- Patsub / casemod on the quoted one-"" join forms ---------------------

#[test]
fn issue3581_array_star_patsub_diagnosed_when_one_empty_quoted() {
    check_bad_substitution("a=(\"\"); echo X\"${a[*]/${-y}/z}\"Y", "${-y}", 1);
}

#[test]
fn issue3581_positional_star_patsub_diagnosed_when_one_empty_quoted() {
    check_bad_substitution("set -- \"\"; echo X\"${*/${-y}/z}\"Y", "${-y}", 1);
}

#[test]
fn issue3581_array_star_casemod_diagnosed_when_one_empty_quoted() {
    check_bad_substitution("a=(\"\"); echo X\"${a[*]^${-y}}\"Y", "${-y}", 1);
}

#[test]
fn issue3581_positional_star_casemod_diagnosed_when_one_empty_quoted() {
    check_bad_substitution("set -- \"\"; echo X\"${*^${-y}}\"Y", "${-y}", 1);
}

// --- Two-"" join forms diagnose (joined word is " ", non-empty) -----------

#[test]
fn issue3581_array_star_two_empty_removal_diagnosed() {
    check_bad_substitution("a=(\"\",\"\"); echo X\"${a[*]#*a=${-y}}\"Y", "*a=${-y}", 1);
}

#[test]
fn issue3581_positional_star_two_empty_removal_diagnosed() {
    check_bad_substitution("set -- \"\" \"\"; echo X\"${*#*a=${-y}}\"Y", "*a=${-y}", 1);
}

// --- Compound-assignment arm diagnoses too --------------------------------

#[test]
fn issue3581_array_star_removal_diagnosed_in_compound_assignment() {
    check_bad_substitution(
        "a=(\"\"); b=(\"${a[*]#*a=${-y}}\"); echo done",
        "*a=${-y}",
        1,
    );
}

#[test]
fn issue3581_positional_star_removal_diagnosed_in_compound_assignment() {
    check_bad_substitution(
        "set -- \"\"; b=(\"${*#*a=${-y}}\"); echo done",
        "*a=${-y}",
        1,
    );
}

// --- Guards: the `[@]` / `@` forms still diagnose -------------------------

#[test]
fn issue3581_array_at_prefix_removal_still_diagnosed() {
    check_bad_substitution("a=(\"\"); echo X\"${a[@]#*a=${-y}}\"Y", "*a=${-y}", 1);
}

#[test]
fn issue3581_positional_at_prefix_removal_still_diagnosed() {
    check_bad_substitution("set -- \"\"; echo X\"${@#*a=${-y}}\"Y", "*a=${-y}", 1);
}

// --- Guards: silence is preserved where GNU is silent ---------------------

#[test]
fn issue3581_array_star_removal_silent_when_unquoted() {
    check_silent("a=(\"\"); echo X${a[*]#*a=${-y}}Y", "XY\n");
}

#[test]
fn issue3581_positional_star_removal_silent_when_unquoted() {
    check_silent("set -- \"\"; echo X${*#*a=${-y}}Y", "XY\n");
}

#[test]
fn issue3581_array_star_removal_silent_when_empty_list() {
    check_silent("a=(); echo X\"${a[*]#*a=${-y}}\"Y", "XY\n");
}

#[test]
fn issue3581_positional_star_removal_silent_when_empty_list() {
    check_silent("set --; echo X\"${*#*a=${-y}}\"Y", "XY\n");
}

#[test]
fn issue3581_array_star_patsub_silent_when_empty_list() {
    check_silent("a=(); echo X\"${a[*]/${-y}/z}\"Y", "XY\n");
}

// --- Guards: valid patterns still work (per-element, then IFS join) -------

#[test]
fn issue3581_array_star_valid_prefix_removal() {
    check_silent("a=(xx hyy); echo X\"${a[*]#h}\"Y", "Xxx yyY\n");
}

#[test]
fn issue3581_array_star_valid_prefix_removal_whole_word() {
    check_silent("a=(hello world); echo \"${a[*]#h}\"", "ello world\n");
}

#[test]
fn issue3581_positional_star_valid_suffix_removal() {
    check_silent("set -- ax bx; echo X\"${*%x}\"Y", "Xa bY\n");
}

#[test]
fn issue3581_array_star_valid_patsub() {
    check_silent("a=(ax bx); echo \"${a[*]/x/y}\"", "ay by\n");
}

#[test]
fn issue3581_array_star_valid_casemod_noop() {
    check_silent("a=(ax bx); echo \"${a[*]^x}\"", "ax bx\n");
}

#[test]
fn issue3581_array_star_valid_removal_unquoted() {
    check_silent("a=(xx hyy); echo X${a[*]#h}Y", "Xxx yyY\n");
}
