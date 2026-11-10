//! Regression pins for rubash#3246 — an arithmetic nameref error does
//! not join the evaluation-order event log: an empty-cell nameref bound
//! to a non-name value (`` `1': not a valid identifier ``) is reported
//! by a post-hoc reporter (arithmetic_aliases.rs) instead of at its
//! production point, so all five shapes diverge from GNU:
//!   n1 `(( ref=1, LC_ALL=5 ))` — GNU diag -> warning, rc=0;
//!     rubash warning -> diag (order reversed), rc=1.
//!   n2 `(( LC_ALL=5, ref=1 ))` — GNU warning -> diag, rc=0;
//!     rubash same order by luck, rc=1 (status hijacked).
//!   n3 `x=$(( ref=1, LC_ALL=5 ))` — GNU diag -> warning, x=5;
//!     rubash diag missing entirely (fast-assignment route never
//!     consumes the marker).
//!   n4 `x=$(( ref=1 ))` then `(( 1 ))` — GNU diag at once, nothing
//!     after; rubash missing at once, the NEXT unrelated command
//!     re-prints the stale diag and fails rc=1 (marker leak).
//!   n5 `let "ref=1" "LC_ALL=5"` — GNU diag -> warning, both operands
//!     evaluated, rc=0; rubash diag only, second operand never
//!     evaluated, rc=1.
//!
//! GNU's rule (variables.c:3076-3079 bind_variable_internal — an
//! empty nameref cell adopting an invalid value fails via
//! sh_invalidid(value), builtins/common.c:208; expr.c:331-354
//! expr_bind_variable does NOT longjmp on the NULL bind, so
//! evaluation continues and the assignment expression still yields
//! its RHS value):
//!   - the diagnostic fires synchronously at the bind point, in
//!     evaluation order with the locale warnings, carrying the
//!     command label (`((:` / `let:`) or no label inside `$(( ))`
//!     (this_command_name is NULL there);
//!   - it is NON-fatal: the command status is the expression value
//!     (`(( ref=1 ))` -> 0, `(( ref=0 ))` -> 1, `let "ref=0"` -> 1),
//!     later comma operands still evaluate, and the marker never
//!     survives into the next command;
//!   - one failed bind = one line: two namerefs in one expression
//!     print two lines, in order, with no dedup.
//!
//! All expectations are the literal GNU bash 5.3.20 outputs for the
//! same script files in a locale-clean environment (only C/C.utf8/
//! POSIX locales installed; the numeric values used here can never
//! resolve to a real locale name), captured 2026-11-11 from probes
//! n1-n5 in ~/workspace/review-3244/probes/ and gnu-status.sh /
//! gnu-extra.sh in ~/workspace/fix-3246/probes/.
#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3246-{tag}-{}", std::process::id()));
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

fn run_script_with_file(
    tag: &str,
    body: &str,
    file: &str,
) -> (String, String, String, Option<i32>, String) {
    let dir = scratch(tag);
    let script = dir.join("probe.sh");
    std::fs::write(&script, body).expect("write script");
    let output = base()
        .arg(&script)
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let captured = std::fs::read_to_string(dir.join(file)).unwrap_or_default();
    let result = (
        script.to_string_lossy().into_owned(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
        captured,
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn issue3246_n1_diag_before_warning() {
    let (path, stdout, stderr, code) = run_script(
        "n1",
        "declare -n ref\n(( ref=1, LC_ALL=5 ))\necho \"rc=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 2: ((: `1': not a valid identifier\n\
             {path}: line 2: warning: setlocale: LC_ALL: cannot change locale (5): No such file or directory\n"
        )
    );
}

#[test]
fn issue3246_n2_warning_before_diag() {
    let (path, stdout, stderr, code) = run_script(
        "n2",
        "declare -n ref\n(( LC_ALL=5, ref=1 ))\necho \"rc=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 2: warning: setlocale: LC_ALL: cannot change locale (5): No such file or directory\n\
             {path}: line 2: ((: `1': not a valid identifier\n"
        )
    );
}

#[test]
fn issue3246_n3_expansion_diag_present() {
    let (path, stdout, stderr, code) = run_script(
        "n3",
        "declare -n ref\nx=$(( ref=1, LC_ALL=5 ))\necho \"rc=$? x=$x\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0 x=5\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 2: `1': not a valid identifier\n\
             {path}: line 2: warning: setlocale: LC_ALL: cannot change locale (5): No such file or directory\n"
        )
    );
}

#[test]
fn issue3246_n4_no_leak_to_next_command() {
    let (path, stdout, stderr, code) = run_script(
        "n4",
        "declare -n ref\nx=$(( ref=1 ))\necho \"x=$x rc=$?\"\n(( 1 ))\necho \"after rc=$?\"\ny=$(( ref=2, LC_ALL=5 ))\necho \"y=$y\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "x=1 rc=0\nafter rc=0\ny=5\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 2: `1': not a valid identifier\n\
             {path}: line 6: `2': not a valid identifier\n\
             {path}: line 6: warning: setlocale: LC_ALL: cannot change locale (5): No such file or directory\n"
        )
    );
}

#[test]
fn issue3246_n5_let_continues_to_next_operand() {
    let (path, stdout, stderr, code) = run_script(
        "n5",
        "declare -n ref\nlet \"ref=1\" \"LC_ALL=5\"\necho \"rc=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 2: let: `1': not a valid identifier\n\
             {path}: line 2: warning: setlocale: LC_ALL: cannot change locale (5): No such file or directory\n"
        )
    );
}

#[test]
fn issue3246_bare_assign_value_decides_status() {
    // The failed bind does not force status 1: the assignment
    // expression still yields its RHS value (GNU expr.c:331-354 —
    // no longjmp on the NULL bind).
    let (path, stdout, stderr, code) = run_script(
        "bare",
        "declare -n ref\n(( ref=1 )); echo \"a=$?\"\n(( ref=0 )); echo \"b=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "a=0\nb=1\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 2: ((: `1': not a valid identifier\n\
             {path}: line 3: ((: `0': not a valid identifier\n"
        )
    );
}

#[test]
fn issue3246_two_namerefs_each_reported() {
    // One failed bind = one line, in order, no dedup (GNU prints
    // both; the old single-slot marker could only ever hold the last).
    let (path, stdout, stderr, code) = run_script(
        "two",
        "declare -n ref\ndeclare -n ref2\n(( ref=1, ref2=2 )); echo \"rc=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 3: ((: `1': not a valid identifier\n\
             {path}: line 3: ((: `2': not a valid identifier\n"
        )
    );
}

#[test]
fn issue3246_let_value_decides_status() {
    let (path, stdout, stderr, code) = run_script(
        "let-status",
        "declare -n ref\nlet \"ref=1\"; echo \"a=$?\"\nlet \"ref=0\"; echo \"b=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "a=0\nb=1\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 2: let: `1': not a valid identifier\n\
             {path}: line 3: let: `0': not a valid identifier\n"
        )
    );
}

#[test]
fn issue3246_expansion_value_kept() {
    // `$(( ref=1 ))` still yields the assigned value, with the
    // diagnostic unlabeled (GNU $(( )): this_command_name NULL).
    let (path, stdout, stderr, code) = run_script(
        "expand-value",
        "declare -n ref\nx=$(( ref=1 )); echo \"x=$x rc=$?\"\nx=$(( ref=0 )); echo \"x=$x rc=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "x=1 rc=0\nx=0 rc=0\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 2: `1': not a valid identifier\n\
             {path}: line 3: `0': not a valid identifier\n"
        )
    );
}

#[test]
fn issue3246_subscript_bind_reports_unlabeled() {
    // A nameref bind failing inside an array subscript reports at
    // the subscript entry's drain, with no command label (GNU
    // arrayfunc.c:1374-1378: this_command_name is NULL inside
    // array_expand_index) — the value still binds as the index.
    let (path, stdout, stderr, code) = run_script(
        "subscript",
        "declare -n ref\na=(x y z)\necho \"v=${a[ref=1]} rc=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "v=y rc=0\n");
    assert_eq!(
        stderr,
        format!("{path}: line 3: `1': not a valid identifier\n")
    );
}

#[test]
fn issue3246_cond_subscript_bind_reports_unlabeled() {
    // Same bind under [[ ]]: the subscript is expanded (and its
    // diagnostic drained) before the conditional operands are
    // evaluated, so the line is still unlabeled and the test itself
    // passes on the bound index.
    let (path, stdout, stderr, code) = run_script(
        "cond-subscript",
        "declare -n ref\na=(x y z)\n[[ ${a[ref=1]} = y ]]; echo \"rc=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(
        stderr,
        format!("{path}: line 3: `1': not a valid identifier\n")
    );
}

#[test]
fn issue3246_nameref_diag_honors_fd2_redirect() {
    // GNU error.c writes diagnostics to fd 2 as currently bound:
    // the `(( ))` command's own `2>file` captures the line. The
    // old post-hoc reporter routed through arith_diag (fd-2-bound);
    // the event-log drain must keep that channel, not leak the line
    // to the shell's outer stderr.
    let (path, stdout, stderr, code, captured) = run_script_with_file(
        "fd2-nameref",
        "declare -n ref\n(( ref=1 )) 2>redir.txt\necho \"rc=$?\"\n",
        "redir.txt",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(
        captured,
        format!("{path}: line 2: ((: `1': not a valid identifier\n")
    );
}

#[test]
fn issue3246_array_diag_honors_fd2_redirect() {
    // Same fd-2 channel for the pre-existing InvalidIdentifier
    // producer (`a[]` bind, rubash#3078): its drain arm previously
    // used a raw eprintln! and bypassed the enclosing redirect.
    let (path, stdout, stderr, code, captured) = run_script_with_file(
        "fd2-array",
        "(( a[]=1 )) 2>redir.txt\necho \"rc=$?\"\n",
        "redir.txt",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(
        captured,
        format!("{path}: line 1: ((: `a[]': not a valid identifier\n")
    );
}

#[test]
fn issue3246_cond_operand_assign_reports_with_label() {
    // A nameref bind failing inside a [[ ]] arithmetic operand
    // reports with the `[[` label (GNU: the operand evaluation runs
    // under the [[ builtin, this_command_name is `[[`) and the
    // comparison still uses the assignment's RHS value.
    let (path, stdout, stderr, code) = run_script(
        "cond-operand",
        "declare -n ref\n[[ ref=1 -eq 1 ]]; echo \"a=$?\"\n[[ ref=0 -eq 0 ]]; echo \"b=$?\"\n[[ ref=1 -eq 0 ]]; echo \"c=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "a=0\nb=0\nc=1\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 2: [[: `1': not a valid identifier\n\
             {path}: line 3: [[: `0': not a valid identifier\n\
             {path}: line 4: [[: `1': not a valid identifier\n"
        )
    );
}

#[test]
fn issue3246_valid_cell_forwards_silently() {
    // Control: a nameref cell holding a valid name forwards the
    // write to the target — no diagnostic, target bound.
    let (_path, stdout, stderr, code) = run_script(
        "valid-cell",
        "declare -n ok=x\n(( ok=7 )); echo \"x=$x rc=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "x=7 rc=0\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue3246_fatal_abort_still_reports_once() {
    // A fatal evalerror after the failed bind aborts the expression
    // but the nameref diagnostic still prints exactly once, before
    // the fatal line (GNU expr.c: the bind diagnostic fires at its
    // point; evalerror at expr.c:1525 aborts afterwards).
    let (path, stdout, stderr, code) = run_script(
        "fatal",
        "declare -n ref\n(( ref=1, 1/0 )); echo \"rc=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 2: ((: `1': not a valid identifier\n\
             {path}: line 2: ((: ref=1, 1/0 : division by 0 (error token is \"0 \")\n"
        )
    );
}

#[test]
fn issue3246_fatal_expansion_discards_command() {
    // The fatal expansion discards the enclosing command (the first
    // echo never runs) while the script continues on the NEXT line —
    // and the nameref diagnostic prints exactly once, unlabeled,
    // before the fatal line. (A follow-up on the SAME line is
    // discarded too, GNU DISCARD; the probe keeps it on the next
    // line so the continuation is observable. `$?` there is 1: the
    // discarded echo's own failure status.)
    let (path, stdout, stderr, code) = run_script(
        "fatal-expand",
        "declare -n ref\necho $(( ref=1, 1/0 ))\necho \"after rc=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after rc=1\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 2: `1': not a valid identifier\n\
             {path}: line 2: ref=1, 1/0 : division by 0 (error token is \"0 \")\n"
        )
    );
}

#[test]
fn issue3246_fatal_fast_assignment_reports_once() {
    // Same fatal abort through the whole-value `$(( ))`
    // fast-assignment route (the n3/n4 route): the assignment is
    // discarded (x stays unset) and the nameref diagnostic prints
    // exactly once, unlabeled, before the fatal line. On the base
    // tree the diagnostic is missing entirely here — the fast route
    // never consumed the marker.
    let (path, stdout, stderr, code) = run_script(
        "fatal-assign",
        "declare -n ref\nx=$(( ref=1, 1/0 ))\necho \"after rc=$? x=[$x]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after rc=1 x=[]\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 2: `1': not a valid identifier\n\
             {path}: line 2: ref=1, 1/0 : division by 0 (error token is \"0 \")\n"
        )
    );
}
