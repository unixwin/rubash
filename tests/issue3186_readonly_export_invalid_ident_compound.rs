//! rubash#3186 — readonly/export/declare accept invalid identifiers in
//! compound operands: GNU diagnoses `` `X': not a valid identifier `` with
//! the PLAIN expanded word as subject, and `declare 'a[]=x'` reports the
//! bare `a[]: bad array subscript`.
//!
//! GNU ground (bash 5.3.20, script-file probes, probe4.sh / probe5*.sh):
//! - `setattr.def` valid_identifier check: every illegal operand reports
//!   `` `name': not a valid identifier ``, rc=1, and the command CONTINUES
//!   to later operands (probe5b q19-q21: `readonly -a 'invalid+ident=(x)'
//!   good` binds `good`, status 1).
//! - The diagnostic subject is the word AFTER normal word expansion:
//!   double-quoted `$x` expands (`b+d6=(a b)`), single-quoted text stays
//!   verbatim (`b+d15=($x)`, `b+d7=("a b" c)`, `b+d14=(a\b c)`).
//! - `declare 'a[]=x'` (also -a/-A, `+=`, typeset, local-in-function)
//!   reports the BARE `a[]: bad array subscript` (arrayfunc.c:359
//!   sublen<=1 -> err_badarraysub, error.c:440-443), rc=1, later operands
//!   still bind; `declare 'a[]='` is the same, `declare 'a[]'` (no `=`)
//!   stays `` `a[]': not a valid identifier ``, `declare -p 'a[]=x'`
//!   stays `a[]=x: not found`, and export/readonly `a[]=x` stay
//!   InvalidId (setattr.def does not split subscripts).
//! - Residual (a), captain to rule: unquoted `declare -a 9bad=(x)` is a
//!   GNU PARSE-time syntax error; rubash reports builtin InvalidId
//!   (same rc=1, different text). The nail locks the InvalidId interim.
//! - Known remaining gap (REPORT §6): GNU also creates the empty array
//!   (`declare -a a=()`) after the bad-subscript failure; this fix only
//!   repairs the diagnostic, not the creation side effect.
//!
//! Expectations byte-verified against the GNU bash 5.3.20 oracle; the
//! script path is normalized to SCRIPT, mirroring tests/issue2462_*.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue3186-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Run `script` as a script FILE; return (stdout, stderr) with the
/// scratch path normalized to WORK and the script path to SCRIPT.
fn run_script(dir: &Path, script: &str) -> (String, String) {
    let script_path = dir.join("probe.sh");
    fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(RUBASH)
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .output()
        .expect("run rubash");
    let normalize = |text: &str| {
        text.replace(&script_path.to_string_lossy().into_owned(), "SCRIPT")
            .replace(&dir.to_string_lossy().into_owned(), "WORK")
    };
    (
        normalize(&String::from_utf8_lossy(&output.stdout)),
        normalize(&String::from_utf8_lossy(&output.stderr)),
    )
}

#[test]
fn readonly_a_invalid_compound_subject_is_plain_word() {
    let dir = scratch("ro-a");
    let (stdout, stderr) = run_script(&dir, "readonly -a 'invalid+ident=(x)'\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(
        stderr,
        "SCRIPT: line 1: readonly: `invalid+ident=(x)': not a valid identifier\n"
    );
}

#[test]
fn export_cap_a_invalid_compound_subject_is_plain_word() {
    let dir = scratch("ex-a");
    let (stdout, stderr) = run_script(&dir, "export -A 'invalid+ident=(k v)'\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(
        stderr,
        "SCRIPT: line 1: export: `invalid+ident=(k v)': not a valid identifier\n"
    );
}

#[test]
fn declare_a_invalid_compound_subject_is_plain_word() {
    let dir = scratch("decl-a");
    let (stdout, stderr) = run_script(&dir, "declare -a 'invalid+ident=(x)'\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(
        stderr,
        "SCRIPT: line 1: declare: `invalid+ident=(x)': not a valid identifier\n"
    );
}

#[test]
fn declare_a_digit_leading_compound_unquoted_is_invalid_id() {
    let dir = scratch("digit");
    let (stdout, stderr) = run_script(&dir, "declare -a 9bad=(x)\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(
        stderr,
        "SCRIPT: line 1: declare: `9bad=(x)': not a valid identifier\n"
    );
}

#[test]
fn readonly_a_invalid_compound_expands_double_quoted_value() {
    let dir = scratch("v6");
    let (stdout, stderr) = run_script(&dir, "x='a b'\nreadonly -a \"b+d6=($x)\"\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(
        stderr,
        "SCRIPT: line 2: readonly: `b+d6=(a b)': not a valid identifier\n"
    );
}

#[test]
fn readonly_a_invalid_compound_keeps_single_quoted_text() {
    let dir = scratch("v7");
    let (stdout, stderr) = run_script(&dir, "readonly -a 'b+d7=(\"a b\" c)'\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(
        stderr,
        "SCRIPT: line 1: readonly: `b+d7=(\"a b\" c)': not a valid identifier\n"
    );
}

#[test]
fn readonly_a_invalid_compound_does_not_expand_single_quoted_dollar() {
    let dir = scratch("v15");
    let (stdout, stderr) = run_script(&dir, "x='a b'\nreadonly -a 'b+d15=($x)'\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(
        stderr,
        "SCRIPT: line 2: readonly: `b+d15=($x)': not a valid identifier\n"
    );
}

#[test]
fn readonly_a_invalid_compound_keeps_backslash_and_subscripts() {
    let dir = scratch("v14");
    let (stdout, stderr) = run_script(&dir, "readonly -a 'b+d14=(a\\b c)'\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(
        stderr,
        "SCRIPT: line 1: readonly: `b+d14=(a\\b c)': not a valid identifier\n"
    );
}

#[test]
fn readonly_a_invalid_compound_indexed_elements_verbatim() {
    let dir = scratch("v9");
    let (stdout, stderr) = run_script(&dir, "readonly -a 'b+d9=([0]=x [5]=y)'\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(
        stderr,
        "SCRIPT: line 1: readonly: `b+d9=([0]=x [5]=y)': not a valid identifier\n"
    );
}

#[test]
fn declare_empty_subscript_bare_diagnostic() {
    let dir = scratch("empty");
    let (stdout, stderr) = run_script(&dir, "declare 'a[]=x'\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(stderr, "SCRIPT: line 1: a[]: bad array subscript\n");
}

#[test]
fn declare_a_empty_subscript_bare_diagnostic() {
    let dir = scratch("empty-a");
    let (stdout, stderr) = run_script(&dir, "declare -a 'a[]=x'\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(stderr, "SCRIPT: line 1: a[]: bad array subscript\n");
}

#[test]
fn declare_cap_a_empty_subscript_bare_diagnostic() {
    let dir = scratch("empty-capa");
    let (stdout, stderr) = run_script(&dir, "declare -A 'a[]=x'\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(stderr, "SCRIPT: line 1: a[]: bad array subscript\n");
}

#[test]
fn typeset_empty_subscript_bare_diagnostic() {
    let dir = scratch("empty-typeset");
    let (stdout, stderr) = run_script(&dir, "typeset 'a[]=x'\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(stderr, "SCRIPT: line 1: a[]: bad array subscript\n");
}

#[test]
fn declare_empty_subscript_assign_empty_value_bare_diagnostic() {
    let dir = scratch("empty-val");
    let (stdout, stderr) = run_script(&dir, "declare 'a[]='\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(stderr, "SCRIPT: line 1: a[]: bad array subscript\n");
}

#[test]
fn declare_empty_subscript_continues_to_later_operands() {
    let dir = scratch("empty-cont");
    let (stdout, stderr) = run_script(
        &dir,
        "declare 'a[]=x' good\necho \"rc=$?\"\ndeclare -p good\n",
    );
    assert_eq!(stdout, "rc=1\ndeclare -- good\n");
    assert_eq!(stderr, "SCRIPT: line 1: a[]: bad array subscript\n");
}

#[test]
fn invalid_compound_continues_to_later_operands() {
    let dir = scratch("compound-cont");
    let (stdout, stderr) = run_script(
        &dir,
        "readonly -a 'invalid+ident=(x)' good\necho \"rc=$?\"\ndeclare -p good\n",
    );
    assert_eq!(stdout, "rc=1\ndeclare -r good\n");
    assert_eq!(
        stderr,
        "SCRIPT: line 1: readonly: `invalid+ident=(x)': not a valid identifier\n"
    );
}

#[test]
fn local_empty_subscript_bare_diagnostic_in_function() {
    let dir = scratch("empty-local");
    let (stdout, stderr) = run_script(&dir, "g() { local 'a[]=x'; echo \"rc=$?\"; }\ng\n");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("a[]: bad array subscript"),
        "stderr should carry the bare bad-subscript diagnostic, got: {stderr}"
    );
    assert!(
        !stderr.contains("not a valid identifier"),
        "stderr must not fall back to InvalidId, got: {stderr}"
    );
}

#[test]
fn declare_p_empty_subscript_stays_not_found() {
    let dir = scratch("p-empty");
    let (stdout, stderr) = run_script(&dir, "declare -p 'a[]=x'\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(stderr, "SCRIPT: line 1: declare: a[]=x: not found\n");
}

#[test]
fn declare_bare_empty_subscript_without_assign_stays_invalid_id() {
    let dir = scratch("noassign");
    let (stdout, stderr) = run_script(&dir, "declare 'a[]'\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(
        stderr,
        "SCRIPT: line 1: declare: `a[]': not a valid identifier\n"
    );
}

#[test]
fn export_empty_subscript_stays_invalid_id() {
    let dir = scratch("ex-empty");
    let (stdout, stderr) = run_script(&dir, "export 'a[]=x'\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(
        stderr,
        "SCRIPT: line 1: export: `a[]': not a valid identifier\n"
    );
}

#[test]
fn readonly_empty_subscript_stays_invalid_id() {
    let dir = scratch("ro-empty");
    let (stdout, stderr) = run_script(&dir, "readonly 'a[]=x'\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(
        stderr,
        "SCRIPT: line 1: readonly: `a[]': not a valid identifier\n"
    );
}

#[test]
fn valid_compound_still_binds() {
    let dir = scratch("valid");
    let (stdout, stderr) = run_script(
        &dir,
        "declare -a 'good=(x y)'\necho \"rc=$?\"\ndeclare -p good\n",
    );
    assert_eq!(stdout, "rc=0\ndeclare -a good=([0]=\"x\" [1]=\"y\")\n");
    assert_eq!(stderr, "");
}
