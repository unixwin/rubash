//! rubash#3501 — a `bad substitution` inside a pattern word must stay silent
//! when the parameter word is absent (and, for `#`/`%`, empty).
//!
//! GNU anchors (bash 5.3, subst.c):
//! - `#`/`%`: `parameter_brace_expand`'s `case '#': case '%':`
//!   (subst.c:10320-10327) breaks out before `parameter_brace_remove_pattern`
//!   when `value == 0 || *value == '\0'` — the pattern is never expanded, so
//!   a bad substitution inside it is never diagnosed.
//! - `/`: `parameter_brace_patsub` (subst.c:9362-9367) returns NULL before
//!   `getpattern` when `value == 0` (unset) — but a set-but-empty value
//!   proceeds, so the pattern IS diagnosed there.
//! - `^`/`,`/`~`: `parameter_brace_casemod` (subst.c:9604-9608) has the same
//!   `if (value == 0) return NULL` gate as patsub.
//! - In all three, the pattern word goes through `getpattern`
//!   (subst.c:5683) -> `expand_string_for_pat` (subst.c:4509), so when the
//!   value IS present a bad substitution inside the pattern is diagnosed
//!   with the PATTERN word as the subject (subst.c:10278 names `string`,
//!   the word `expand_word_internal` was expanding).
//!
//! Expectations verified against GNU bash 5.3.20 (local oracle probes;
//! harness `~/workspace/fix-3501/oracle-verify-pin.py` replays every
//! cell against the oracle). Each case runs as a one-line script so the
//! diagnostic is `rubash: line 1: <word>: bad substitution`. Silent
//! cases assert the exact stdout, rc 0 and empty stderr; error cases
//! assert rc 1, exactly one diagnostic, and the exact word it names.
//!
//! Known residual (NOT covered here): a QUOTED `[@]`/`[*]`/`@` word
//! with exactly one "" element diagnoses in GNU (the value word carries
//! the QUOTED_NULL marker, subst.c:10133-10140 + the `#` case `*temp`
//! test at subst.c:10320), but rubash's braced expansion does not thread
//! quoting into the removal gate, so it stays silent there. The compound
//! -assignment arm pins the quoted behavior (it knows the word is
//! quoted); the direct `"${a[@]#...}"` form awaits quoted-flag threading
//! and is filed as a follow-up, not papered over here.

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3501-{tag}-{}", std::process::id()));
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

/// The script must not report any `bad substitution` and must exit 0.
/// Stdout is not asserted: the compound-assignment element count for an
/// empty `[@]` word is a separate pre-existing shape, not this issue.
fn check_no_bad_substitution(script: &str) {
    let (stdout, stderr, rc) = run_script(script);
    assert_eq!(rc, 0, "script {script:?}: rc {rc}, stderr: {stderr}");
    assert!(
        !stderr.contains("bad substitution"),
        "script {script:?}: unexpected bad substitution, stderr: {stderr}, stdout: {stdout}"
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

// --- Silent: parameter unset -----------------------------------------------

#[test]
fn issue3501_patsub_pattern_silent_when_unset() {
    check_silent("echo X${x/a=${-y}/z}Y", "XY\n");
}

#[test]
fn issue3501_prefix_removal_pattern_silent_when_unset() {
    check_silent("echo X${x#*a=${-y}}Y", "XY\n");
}

#[test]
fn issue3501_suffix_removal_pattern_silent_when_unset() {
    check_silent("echo X${x%*a=${-y}}Y", "XY\n");
}

#[test]
fn issue3501_patsub_pattern_only_silent_when_unset() {
    check_silent("echo X${x/${-y}/z}Y", "XY\n");
}

#[test]
fn issue3501_patsub_replacement_silent_when_unset() {
    check_silent("echo X${x/a/${-y}}Y", "XY\n");
}

#[test]
fn issue3501_casemod_upper_silent_when_unset() {
    check_silent("echo X${x^${-y}}Y", "XY\n");
}

#[test]
fn issue3501_casemod_lower_silent_when_unset() {
    check_silent("echo X${x,${-y}}Y", "XY\n");
}

#[test]
fn issue3501_positional_unset_patsub_silent() {
    check_silent("echo X${1/${-y}/z}Y", "XY\n");
}

#[test]
fn issue3501_positional_unset_removal_silent() {
    check_silent("echo X${1#*a=${-y}}Y", "XY\n");
}

#[test]
fn issue3501_at_list_empty_removal_silent() {
    check_silent("echo X${@#*a=${-y}}Y", "XY\n");
}

#[test]
fn issue3501_at_list_empty_patsub_silent() {
    check_silent("echo X${@/${-y}/z}Y", "XY\n");
}

#[test]
fn issue3501_array_empty_removal_silent() {
    check_silent("a=(); echo X${a[@]#*a=${-y}}Y", "XY\n");
}

#[test]
fn issue3501_array_empty_patsub_silent() {
    check_silent("a=(); echo X${a[@]/${-y}/z}Y", "XY\n");
}

#[test]
fn issue3501_array_element_unset_removal_silent() {
    check_silent("echo X${a[0]#*a=${-y}}Y", "XY\n");
}

#[test]
fn issue3501_array_element_unset_patsub_silent() {
    check_silent("echo X${a[0]/${-y}/z}Y", "XY\n");
}

#[test]
fn issue3501_indirect_missing_patsub_silent() {
    check_silent("x=missing; echo X${!x/a=${-y}/z}Y", "XY\n");
}

#[test]
fn issue3501_indirect_missing_removal_silent() {
    check_silent("x=missing; echo X${!x#*a=${-y}}Y", "XY\n");
}

#[test]
fn issue3501_indirect_missing_casemod_silent() {
    check_silent("x=missing; echo X${!x^${-y}}Y", "XY\n");
}

// --- Silent: `#`/`%` also skip when the value is empty ---------------------

#[test]
fn issue3501_removal_pattern_silent_when_empty() {
    check_silent("x=; echo X${x#*a=${-y}}Y", "XY\n");
}

#[test]
fn issue3501_suffix_removal_pattern_silent_when_empty() {
    check_silent("x=; echo X${x%*a=${-y}}Y", "XY\n");
}

#[test]
fn issue3501_positional_empty_removal_silent() {
    check_silent("set -- \"\"; echo X${1#*a=${-y}}Y", "XY\n");
}

#[test]
fn issue3501_array_element_empty_removal_silent() {
    check_silent("a=(\"\"); echo X${a[0]#*a=${-y}}Y", "XY\n");
}

#[test]
fn issue3501_indirect_empty_removal_silent() {
    check_silent("x=e; e=; echo X${!x#*a=${-y}}Y", "XY\n");
}

#[test]
fn issue3501_scalar_base_array_removal_silent_when_empty() {
    // A scalar base under `[@]` is a one-element list holding the scalar,
    // so `x=""` joins to "" and the removal gate stays silent.
    check_silent("x=; echo X${x[@]#*a=${-y}}Y", "XY\n");
}

#[test]
fn issue3501_assoc_array_one_empty_removal_silent() {
    check_silent("declare -A A=([k]=\"\"); echo X${A[@]#*a=${-y}}Y", "XY\n");
}

#[test]
fn issue3501_indirect_array_one_empty_removal_silent() {
    check_silent("x='a[@]'; a=(\"\"); echo X${!x#*a=${-y}}Y", "XY\n");
}

// --- Silent: secondary entry points (compound assignment / array words) ---

#[test]
fn issue3501_assign_at_patsub_silent_when_no_params() {
    check_no_bad_substitution("a=(\"${@/${-y}/z}\"); echo done");
}

#[test]
fn issue3501_assign_array_empty_patsub_silent() {
    check_no_bad_substitution("a=(); b=(\"${a[@]/${-y}/z}\"); echo done");
}

#[test]
fn issue3501_assign_array_empty_removal_silent() {
    check_no_bad_substitution("a=(); b=(\"${a[@]#*a=${-y}}\"); echo done");
}

#[test]
fn issue3501_assign_scalar_unset_patsub_silent() {
    check_no_bad_substitution("b=(\"${x/${-y}/z}\"); echo done");
}

#[test]
fn issue3501_assign_array_one_empty_removal_diagnosed() {
    // Compound-assignment entry point: this arm handles the QUOTED
    // `[@]` word, and GNU diagnoses a quoted one-element "" list —
    // the value word carries the QUOTED_NULL marker, so the removal
    // gate's `*temp == '\0'` test (subst.c:10320) does not fire.
    // (The UNQUOTED form is silent; see the joined-word cells above.)
    check_bad_substitution(
        "a=(\"\"); b=(\"${a[@]#*a=${-y}}\"); echo done",
        "*a=${-y}",
        1,
    );
}

// --- Still diagnosing: value present --------------------------------------

#[test]
fn issue3501_patsub_pattern_diagnosed_when_set() {
    check_bad_substitution("x=hello; echo X${x/a=${-y}/z}Y", "a=${-y}", 1);
}

#[test]
fn issue3501_prefix_removal_pattern_diagnosed_when_set() {
    check_bad_substitution("x=hello; echo X${x#*a=${-y}}Y", "*a=${-y}", 1);
}

#[test]
fn issue3501_suffix_removal_pattern_diagnosed_when_set() {
    check_bad_substitution("x=hello; echo X${x%*a=${-y}}Y", "*a=${-y}", 1);
}

#[test]
fn issue3501_patsub_pattern_diagnosed_when_empty() {
    check_bad_substitution("x=; echo X${x/a=${-y}/z}Y", "a=${-y}", 1);
}

#[test]
fn issue3501_patsub_replacement_diagnosed_when_empty() {
    check_bad_substitution("x=; echo X${x/a/${-y}}Y", "${-y}", 1);
}

#[test]
fn issue3501_patsub_pattern_only_diagnosed_when_set() {
    check_bad_substitution("x=hello; echo X${x/${-y}/z}Y", "${-y}", 1);
}

#[test]
fn issue3501_casemod_diagnosed_when_set() {
    check_bad_substitution("x=hello; echo X${x^${-y}}Y", "${-y}", 1);
}

#[test]
fn issue3501_casemod_diagnosed_when_empty() {
    check_bad_substitution("x=; echo X${x^${-y}}Y", "${-y}", 1);
}

#[test]
fn issue3501_at_list_one_empty_removal_silent() {
    // Corrected 2026-11-14: the first lane expected a diagnosis here, but
    // GNU is silent — the removal gate reads the JOINED value word
    // (subst.c:10320), and one "" element joins to "" (oracle-verified).
    check_silent("set -- \"\"; echo X${@#*a=${-y}}Y", "XY\n");
}

#[test]
fn issue3501_array_one_empty_removal_silent() {
    // Same correction as above, for the `a[@]` subject.
    check_silent("a=(\"\"); echo X${a[@]#*a=${-y}}Y", "XY\n");
}

#[test]
fn issue3501_array_element_empty_patsub_diagnosed() {
    check_bad_substitution("a=(\"\"); echo X${a[0]/${-y}/z}Y", "${-y}", 1);
}

#[test]
fn issue3501_indirect_empty_patsub_diagnosed() {
    check_bad_substitution("x=e; e=; echo X${!x/${-y}/z}Y", "${-y}", 1);
}

#[test]
fn issue3501_indirect_set_patsub_diagnosed() {
    check_bad_substitution("x=e; e=hello; echo X${!x/a=${-y}/z}Y", "a=${-y}", 1);
}

#[test]
fn issue3501_special_question_removal_diagnosed() {
    check_bad_substitution("echo X${?#*a=${-y}}Y", "*a=${-y}", 1);
}

#[test]
fn issue3501_pattern_word_with_trailing_text_diagnosed() {
    check_bad_substitution("x=hello; echo X${x/a=${-y}b/z}Y", "a=${-y}b", 1);
}

#[test]
fn issue3501_positional_set_removal_diagnosed() {
    check_bad_substitution("set -- hello; echo X${1#*a=${-y}}Y", "*a=${-y}", 1);
}

#[test]
fn issue3501_scalar_base_array_removal_diagnosed_when_set() {
    check_bad_substitution("x=hello; echo X${x[@]#*a=${-y}}Y", "*a=${-y}", 1);
}

#[test]
fn issue3501_assoc_array_two_empty_removal_diagnosed() {
    // Two "" elements join to " " (the IFS separator), so the joined
    // value word is non-empty and the pattern IS expanded.
    check_bad_substitution(
        "declare -A A=([k]=\"\" [j]=\"\"); echo X${A[@]#*a=${-y}}Y",
        "*a=${-y}",
        1,
    );
}

#[test]
fn issue3501_at_list_two_empty_removal_diagnosed() {
    check_bad_substitution("set -- \"\" \"\"; echo X${@#*a=${-y}}Y", "*a=${-y}", 1);
}

#[test]
fn issue3501_array_mixed_removal_diagnosed() {
    check_bad_substitution("a=(\"\" hello); echo X${a[@]#*a=${-y}}Y", "*a=${-y}", 1);
}

#[test]
fn issue3501_at_list_one_empty_patsub_diagnosed() {
    // patsub only tests `value == 0`: one "" element is a present
    // (quoted-null) value, so the pattern IS expanded here — unlike
    // the `#`/`%` gate above, which reads the joined word.
    check_bad_substitution("set -- \"\"; echo X${@/${-y}/z}Y", "${-y}", 1);
}

// --- Guards: pattern expansion still runs when the value is present ------

#[test]
fn issue3501_valid_pattern_still_expands_in_patsub() {
    check_silent("x=hello; y=ell; echo ${x/${y}/ELL}", "hELLo\n");
}

#[test]
fn issue3501_valid_pattern_with_nested_param_still_expands() {
    check_silent("x=hello; y=l; echo ${x/a=${y}/z}", "hello\n");
}

#[test]
fn issue3501_valid_prefix_removal_still_works() {
    check_silent("x=hello; echo ${x#h}", "ello\n");
}

#[test]
fn issue3501_valid_suffix_removal_still_works() {
    check_silent("x=hello; echo ${x%o}", "hell\n");
}

#[test]
fn issue3501_valid_casemod_still_works() {
    check_silent("x=hello; echo ${x^h}", "Hello\n");
}

#[test]
fn issue3501_valid_array_removal_still_works() {
    check_silent("a=(hello world); echo ${a[@]#h}", "ello world\n");
}

// --- Extra boundaries: scalar-base `[0]`, assoc element, nameref ----------

#[test]
fn issue3501_scalar_base_element_removal_silent_when_empty() {
    check_silent("x=; echo X${x[0]#*a=${-y}}Y", "XY\n");
}

#[test]
fn issue3501_assoc_element_unset_patsub_silent() {
    check_silent("declare -A A; echo X${A[k]/a=${-y}/z}Y", "XY\n");
}

#[test]
fn issue3501_nameref_to_unset_patsub_silent() {
    check_silent("declare -n r=missing; echo X${r/a=${-y}/z}Y", "XY\n");
}

#[test]
fn issue3501_scalar_base_element_patsub_diagnosed_when_set() {
    check_bad_substitution("x=hello; echo X${x[0]/a=${-y}/z}Y", "a=${-y}", 1);
}

#[test]
fn issue3501_scalar_base_element_removal_diagnosed_when_set() {
    check_bad_substitution("x=hello; echo X${x[0]#*a=${-y}}Y", "*a=${-y}", 1);
}

#[test]
fn issue3501_scalar_base_element_patsub_diagnosed_when_empty() {
    check_bad_substitution("x=; echo X${x[0]/a=${-y}/z}Y", "a=${-y}", 1);
}

#[test]
fn issue3501_nameref_to_set_patsub_diagnosed() {
    check_bad_substitution(
        "declare -n r=x; x=hello; echo X${r/a=${-y}/z}Y",
        "a=${-y}",
        1,
    );
}

#[test]
fn issue3501_assoc_element_set_patsub_diagnosed() {
    check_bad_substitution(
        "declare -A A=([k]=hello); echo X${A[k]/a=${-y}/z}Y",
        "a=${-y}",
        1,
    );
}

#[test]
fn issue3501_indirect_array_empty_removal_silent() {
    // `${!x}` for an array target is element [0]; an empty array has
    // no element 0, so the value word is absent.
    check_silent("x=a; a=(); echo X${!x#*a=${-y}}Y", "XY\n");
}

#[test]
fn issue3501_indirect_first_element_empty_removal_silent() {
    // Same element-[0] rule: element 0 is "", so the removal gate
    // reads an empty word even though the array has more elements.
    check_silent("x=a; a=(\"\" hello); echo X${!x#*a=${-y}}Y", "XY\n");
}
