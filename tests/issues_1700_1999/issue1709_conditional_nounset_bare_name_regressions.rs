//! rubash#1709 — under `set -u`, a bare unset name as a `[[ ]]`
//! numeric-primary operand was silently evaluated as 0; GNU aborts
//! with the unbound-variable diagnostic.
//!
//! GNU ground truth: test.c:357 arithcomp evaluates both operands via
//! evalexp; expr.c:1190-1216 expr_streval raises err_unboundvar for an
//! unset variable reference when unbound_vars_is_error is set. This is
//! the residual subform of the closed #1130/#1201 family (those covered
//! the `$name` expansion path; a bare name reaches the evaluator
//! without ever expanding). Expectations verified against GNU bash
//! 5.3.20 (script files): diagnostic on stderr, the shell exits 1, and
//! nothing after the conditional runs — but a short-circuited operand
//! is never evaluated and stays silent.

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn run_script(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash1709-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let result = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn issue1709_bare_name_left_operand_aborts() {
    let (stdout, stderr, code) = run_script(
        "left",
        "set -u\n[[ u -eq 0 ]]; echo SHOULD_NOT_PRINT\necho after\n",
    );
    assert_eq!(stdout, "");
    assert!(stderr.contains("u: unbound variable"), "stderr: {stderr}");
    assert_eq!(code, Some(1));
}

#[test]
fn issue1709_bare_name_sibling_operators_and_side() {
    for (tag, line) in [
        ("right-lt", "[[ 0 -lt u ]]; echo SHOULD_NOT_PRINT"),
        ("ne", "[[ u -ne 1 ]]; echo SHOULD_NOT_PRINT"),
        ("ge", "[[ u -ge 0 ]]; echo SHOULD_NOT_PRINT"),
        ("quoted", "[[ 'u' -eq 0 ]]; echo SHOULD_NOT_PRINT"),
        ("or-context", "[[ u -eq 0 ]] || echo SHOULD_NOT_PRINT"),
    ] {
        let (stdout, stderr, code) = run_script(tag, &format!("set -u\n{line}\necho after\n"));
        assert_eq!(stdout, "", "{tag}");
        assert!(
            stderr.contains("u: unbound variable"),
            "{tag} stderr: {stderr}"
        );
        assert_eq!(code, Some(1), "{tag}");
    }
}

#[test]
fn issue1709_short_circuit_and_bound_names_stay_quiet() {
    let (stdout, stderr, code) = run_script(
        "short",
        "set -u\n\
         x=5\n\
         [[ x -eq 6 && u -eq 0 ]]; echo \"short-rc=$?\"\n\
         u=3\n\
         [[ u -eq 3 ]]; echo \"set-rc=$?\"\n\
         echo after\n",
    );
    assert_eq!(stdout, "short-rc=1\nset-rc=0\nafter\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1709_no_nounset_reads_zero_and_dollar_form_parity() {
    // Without set -u an unset bare name still reads as 0.
    let (stdout, _, code) = run_script("off", "[[ u -eq 0 ]]; echo \"rc=$?\"\n");
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(code, Some(0));

    // The $u form covered by #1130/#1201 aborts identically.
    let (stdout, stderr, code) = run_script(
        "dollar",
        "set -u\n[[ $u -eq 0 ]]; echo SHOULD_NOT_PRINT\necho after\n",
    );
    assert_eq!(stdout, "");
    assert!(stderr.contains("u: unbound variable"), "stderr: {stderr}");
    assert_eq!(code, Some(1));
}
