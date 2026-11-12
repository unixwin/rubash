//! Regression pins for rubash#3382 — a fatal `$(( ))` inside a
//! `[[ ]]` operand leaks the arithmetic error flags past the `[[ ]]`
//! command boundary: the NEXT command is discarded by
//! `abort_on_expansion_errors` instead of running (same mechanism
//! family as #3287, but the `[[ ]]` wrapper never consumed the flags
//! at all — `[[ ]]` commands never call `abort_on_expansion_errors`,
//! so the flags even survive a following `[[ ]]`).
//!
//! GNU's rule (expr.c `evalerror` prints the diagnostic then
//! `sh_longjmp(evalbuf, 1)`; the setjmp catch in `evalexp`
//! (expr.c:429-445) returns 0; the failed arithmetic expansion
//! yields `&expand_wdesc_error` and the caller jumps DISCARD —
//! eval.c:111 reader_loop abandons the rest of the CURRENT logical
//! line only; execute_cmd.c:4132 `execute_cond_command` runs the
//! test, and there is no cross-command error state at all):
//!   - the `[[ ]]` test itself fails (status 1) with exactly one
//!     diagnostic, the rest of its logical line is abandoned
//!     (`&&`/`||` branches and same-line followers never run),
//!   - the NEXT line starts clean — the next command runs normally
//!     (echo, `[[ ]]`, `(( ))`, `let`, assignment all evaluate),
//!   - `set -e` does NOT exit on the DISCARD itself (the evalerror
//!     class bypasses the errexit epilogue, rubash#1518),
//!   - but the exemption ends with the DISCARD: a LATER failure
//!     under `set -e` still exits (the flag is consumed when the
//!     list loop absorbs the DISCARD),
//!   - `set -u` nounset inside a `[[ ]]` operand is still
//!     script-fatal (its own FORCE_EOF path, untouched here).
//!
//! The fix consumes the three arith-error flags as ONE error record
//! at the `[[ ]]` command boundary (the with-redirects wrapper in
//! command_prepare.rs and the late-dispatch arm in
//! command_dispatch_late.rs, via the shared
//! `take_conditional_arith_evalerror` helper in conditional.rs) and
//! returns the DISCARD class (`ExpansionFailure(1)`), and consumes
//! `evalerror_arith_pending` when the list loop's DISCARD arm
//! absorbs the jump (ast_exec.rs).
//!
//! GNU expectations below are the literal GNU bash 5.3.20 outputs
//! recorded against the oracle binary
//! (`~/workspace/rubash-audit/gnu-bash-5.3.20/bin/bash`, same script
//! files, 2026-11-12); rubash's diagnostic text is byte-identical to
//! GNU's in every case (verified against the pre-fix base binary).
#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3382-{tag}-{}", std::process::id()));
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

fn div_by_zero(path: &str, line: u32) -> String {
    format!("{path}: line {line}: 1/0 : division by 0 (error token is \"0 \")\n")
}

#[test]
fn issue3382_canary_next_command_runs() {
    // The issue's canary: GNU prints `after` after the diagnostic;
    // the pre-fix binary discards the echo (rc=1, empty stdout).
    let (path, stdout, stderr, code) = run_script(
        "canary",
        "[[ $(( 1/0 )) = x ]]\necho after\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after\n");
    assert_eq!(stderr, div_by_zero(&path, 1));
}

#[test]
fn issue3382_bracket_status_is_one() {
    // The error makes the test fail, not "0=0 true": GNU reports
    // status 1 for the `[[ ]]` itself. Pre-fix the echo that reads
    // `$?` was itself discarded, so nothing printed at all.
    let (path, stdout, stderr, code) = run_script(
        "status1",
        "[[ $(( 1/0 )) = 0 ]]\necho \"rc=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(stderr, div_by_zero(&path, 1));
}

#[test]
fn issue3382_same_line_rest_discarded_next_line_runs() {
    // DISCARD abandons the rest of the logical line (`same-line`
    // never prints) while the next line runs. This was already
    // correct pre-fix (the stale flags discarded the same-line echo
    // for the wrong reason); it pins the line boundary the fix must
    // preserve.
    let (path, stdout, stderr, code) = run_script(
        "sameline",
        "[[ $(( 1/0 )) = x ]]; echo same-line\necho next\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "next\n");
    assert_eq!(stderr, div_by_zero(&path, 1));
}

#[test]
fn issue3382_and_chain_branch_skipped_after_runs() {
    // `[[ ... ]] && echo AND` — the AND branch is on the discarded
    // line, `echo after` on the next line runs. Pre-fix both were
    // discarded (empty stdout, rc=1).
    let (path, stdout, stderr, code) = run_script(
        "andchain",
        "[[ $(( 1/0 )) = x ]] && echo AND\necho after\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after\n");
    assert_eq!(stderr, div_by_zero(&path, 1));
}

#[test]
fn issue3382_or_chain_branch_skipped_after_runs() {
    // `[[ ... ]] || echo OR` — the OR branch is on the discarded
    // line, `echo after` on the next line runs. Already correct
    // pre-fix (the stale flags discarded the OR echo and were
    // consumed by it); it pins the `||` line boundary.
    let (path, stdout, stderr, code) = run_script(
        "orchain",
        "[[ $(( 1/0 )) = x ]] || echo OR\necho after\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after\n");
    assert_eq!(stderr, div_by_zero(&path, 1));
}

#[test]
fn issue3382_if_condition_neither_branch_runs() {
    // The `[[ ]]` is the if-condition: neither branch runs, the
    // command after the `fi` runs. Already correct pre-fix (the
    // ExpansionFailure unwinds through `if_condition_matches`); it
    // pins that the fix keeps the condition path intact.
    let (path, stdout, stderr, code) = run_script(
        "ifcond",
        "if [[ $(( 1/0 )) = x ]]; then echo THEN; else echo ELSE; fi\necho after\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after\n");
    assert_eq!(stderr, div_by_zero(&path, 1));
}

#[test]
fn issue3382_set_e_discard_does_not_exit() {
    // rubash#1518: the arithmetic evalerror DISCARD bypasses the
    // errexit epilogue — under `set -e` the script continues on the
    // next line. Pre-fix the script exited 1 with no `after-e`.
    let (path, stdout, stderr, code) = run_script(
        "sete",
        "set -e\n[[ $(( 1/0 )) = x ]]\necho after-e\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after-e\n");
    assert_eq!(stderr, div_by_zero(&path, 2));
}

#[test]
fn issue3382_set_e_later_failure_still_exits() {
    // The exemption ends with the DISCARD: a LATER failure under
    // `set -e` still exits. Pre-fix this shape was accidentally
    // right for the wrong reason (`false` was discarded by the stale
    // flags, so the script ended rc=1 without `false` ever running);
    // with the wrapper fix alone (no DISCARD-absorb consumption)
    // `false` would run but be wrongly exempted and `after-e` would
    // print. This pin is what justifies the ast_exec.rs consumption.
    let (path, stdout, stderr, code) = run_script(
        "sete-false",
        "set -e\n[[ $(( 1/0 )) = x ]]\nfalse\necho after-e\n",
    );
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "");
    assert_eq!(stderr, div_by_zero(&path, 2));
}

#[test]
fn issue3382_set_u_nounset_still_fatal() {
    // Guardrail: `set -u` nounset inside a `[[ ]]` operand is
    // script-fatal (FORCE_EOF) — the fix must NOT convert it to a
    // line-skip. Both GNU and the pre-fix binary exit 1 here with
    // no `after-u`.
    let (path, stdout, stderr, code) = run_script(
        "setu",
        "set -u\n[[ $(( b )) = x ]]\necho after-u\n",
    );
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "");
    assert_eq!(stderr, format!("{path}: line 2: b: unbound variable\n"));
}

#[test]
fn issue3382_right_operand_fatal_leak() {
    // The fatal expansion on the RIGHT operand leaks the same way.
    // Pre-fix `echo after` was discarded (rc=1, empty stdout).
    let (path, stdout, stderr, code) = run_script(
        "rightop",
        "[[ x = $(( 1/0 )) ]]\necho after\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after\n");
    assert_eq!(stderr, div_by_zero(&path, 1));
}

#[test]
fn issue3382_negated_fatal_leak() {
    // The negated form `[[ ! ... ]]` leaks the same way.
    // Pre-fix `echo after` was discarded (rc=1, empty stdout).
    let (path, stdout, stderr, code) = run_script(
        "negated",
        "[[ ! $(( 1/0 )) = x ]]\necho after\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after\n");
    assert_eq!(stderr, div_by_zero(&path, 1));
}

#[test]
fn issue3382_double_cond_each_prints_once_after_runs() {
    // Two fatal `[[ ]]` commands in a row: each prints exactly one
    // diagnostic, then `echo after` runs. Pre-fix the second `[[ ]]`
    // never evaluated and `after` never printed.
    let (path, stdout, stderr, code) = run_script(
        "double",
        "[[ $(( 1/0 )) = x ]]\n[[ $(( 1/0 )) = y ]]\necho after\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after\n");
    assert_eq!(stderr, div_by_zero(&path, 1) + &div_by_zero(&path, 2));
}

#[test]
fn issue3382_next_bracket_runs_and_reports_clean_status() {
    // The flags survive a following `[[ ]]` pre-fix (no
    // `abort_on_expansion_errors` on that path): `[[ 1 = 1 ]]`
    // evaluated, but the echo reading `$?` was discarded, so nothing
    // printed. After the fix the next `[[ ]]` runs clean (rc=0).
    let (path, stdout, stderr, code) = run_script(
        "nextbracket",
        "[[ $(( 1/0 )) = x ]]\n[[ 1 = 1 ]]\necho \"rc=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, div_by_zero(&path, 1));
}

#[test]
fn issue3382_next_arith_command_runs() {
    // A following `(( ))` is skipped pre-fix (stale fatal flag
    // makes its evaluation return None), so `$?` reads 1. After
    // the fix it evaluates normally (rc=0).
    let (path, stdout, stderr, code) = run_script(
        "nextarith",
        "[[ $(( 1/0 )) = x ]]\n(( 1 ))\necho \"rc=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, div_by_zero(&path, 1));
}

#[test]
fn issue3382_next_let_runs() {
    // `let` takes the same evaluator entry as `(( ))`, so the same
    // stale-flag skip hit it pre-fix. After the fix it evaluates
    // normally (rc=0).
    let (path, stdout, stderr, code) = run_script(
        "nextlet",
        "[[ $(( 1/0 )) = x ]]\nlet x=1\necho \"rc=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, div_by_zero(&path, 1));
}

#[test]
fn issue3382_following_assignment_evaluates() {
    // A following assignment's RHS was discarded pre-fix (`x` stayed
    // empty). After the fix it evaluates normally (x=5).
    let (path, stdout, stderr, code) = run_script(
        "nextassign",
        "[[ $(( 1/0 )) = x ]]\nx=$(( 2 + 3 ))\necho \"x=$x\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "x=5\n");
    assert_eq!(stderr, div_by_zero(&path, 1));
}

#[test]
fn issue3382_assign_then_cond_control() {
    // Control case for the #3287 path: a fatal ASSIGNMENT already
    // consumes its own flags, so the following `[[ ]]` runs clean.
    // Already correct pre-fix; it pins that the fix does not disturb
    // the assignment-side consumption.
    let (path, stdout, stderr, code) = run_script(
        "assignthencond",
        "x=$(( 1/0 ))\n[[ 1 = 1 ]]\necho \"rc=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, div_by_zero(&path, 1));
}
