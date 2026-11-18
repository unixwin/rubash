//! Regression pins for rubash#3456: a pipeline element's prefix
//! assignment ignores a fatal arithmetic error.
//!
//! GNU rule (bash 5.2.21 oracle on this machine; the issue cites 5.3.20
//! with the same behaviour): a fatal `$(( ))` error while expanding a
//! pipeline element's `VAR=value` prefix prints the diagnostic exactly
//! once, discards the WHOLE element (it never runs, and for an external
//! stage the child never spawns), and folds the element's status to 1.
//! That holds for every element position, builtin and external stages,
//! function stages, `set -o pipefail`, background pipelines, and
//! command substitution.
//!
//! Two faces on the base head (8100d767):
//!   A — builtin/stand-in stage (`echo`, `printf`, `true`, `:`,
//!       `false`, `cat`, `grep`, `wc`, `tr`, `head`, `trap -p`): the
//!       error was swallowed entirely — no diagnostic, the stage ran,
//!       status 0. The inline slot arms in
//!       `execute_pipeline_stage_inner` never expanded the prefix
//!       assignments at all.
//!   B — external stage (`printenv`, `sh -c`, `wc -l /dev/null`,
//!       `yes | head -1`): the diagnostic printed and the status folded
//!       to 1, but the child still spawned and its output leaked.
//!
//! Two corrections to the triage summary, both re-probed against GNU
//! 5.2.21 while writing this stem:
//!   - `set -u` nounset inside `$(( ))` in a pipeline prefix is status
//!     **1**, not 127 (`x=$undef echo hi | cat` is `pipe=1 0` on GNU,
//!     and so is `echo $undef | cat`); the non-pipeline shape instead
//!     kills the whole script with rc 1.
//!   - `x=$(x=$((1/0)) echo hi)` reports `rc=1`, not 0 (the comsub's
//!     status is the inner command's status).
//!
//! Delimitations, all GNU-oracle-verified:
//!   - a prefix before a compound command is a syntax error on GNU too
//!     (rc 2); `x=hello echo "stage x=[$x]"` prints `stage x=[]` on
//!     GNU too (words expand before the prefix binds).
//!   - `x=$((1[0]=2))`: rubash's diagnostic reads `arithmetic syntax
//!     error: invalid arithmetic operator` where GNU reads `syntax
//!     error: invalid arithmetic operator` — a pre-existing evaluator
//!     wording divergence, pinned here as rubash prints it, not part
//!     of this fix.
//!
//! GNU expectations below are the literal GNU bash 5.2.21 outputs for
//! these scripts (oracle binary: the system bash, LC_ALL=C.UTF-8).
#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3456-{tag}-{}", std::process::id()));
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

fn div0(path: &str, line: u32) -> String {
    format!("{path}: line {line}: 1/0: division by 0 (error token is \"0\")\n")
}

#[test]
fn issue3456_prefix_arith_echo_first_element() {
    let (path, stdout, stderr, code) = run_script(
        "q1-echo",
        "x=$((1/0)) echo hi | cat\necho \"pipe=${PIPESTATUS[*]}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "pipe=1 0\n");
    assert_eq!(stderr, div0(&path, 1));
}

#[test]
fn issue3456_prefix_arith_printf_first_element() {
    let (path, stdout, stderr, code) = run_script(
        "q2-printf",
        "x=$((1/0)) printf \"hi\\n\" | cat\necho \"pipe=${PIPESTATUS[*]}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "pipe=1 0\n");
    assert_eq!(stderr, div0(&path, 1));
}

#[test]
fn issue3456_prefix_arith_true_first_element() {
    let (path, stdout, stderr, code) = run_script(
        "q3-true",
        "x=$((1/0)) true | cat\necho \"pipe=${PIPESTATUS[*]}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "pipe=1 0\n");
    assert_eq!(stderr, div0(&path, 1));
}

#[test]
fn issue3456_prefix_arith_colon_first_element() {
    let (path, stdout, stderr, code) = run_script(
        "q4-colon",
        "x=$((1/0)) : | cat\necho \"pipe=${PIPESTATUS[*]}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "pipe=1 0\n");
    assert_eq!(stderr, div0(&path, 1));
}

#[test]
fn issue3456_prefix_arith_false_first_element() {
    // `false` is status 1 on its own, so the pipe status alone does not
    // discriminate — the diagnostic is the pin here.
    let (path, stdout, stderr, code) = run_script(
        "q5-false",
        "x=$((1/0)) false | cat\necho \"pipe=${PIPESTATUS[*]}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "pipe=1 0\n");
    assert_eq!(stderr, div0(&path, 1));
}

#[test]
fn issue3456_prefix_arith_cat_second_element() {
    let (path, stdout, stderr, code) = run_script(
        "q6-cat2",
        "echo hello | x=$((1/0)) cat\necho \"pipe=${PIPESTATUS[*]}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "pipe=0 1\n");
    assert_eq!(stderr, div0(&path, 1));
}

#[test]
fn issue3456_prefix_arith_grep_second_element() {
    let (path, stdout, stderr, code) = run_script(
        "q7-grep",
        "printf \"foo\\n\" | x=$((1/0)) grep foo\necho \"pipe=${PIPESTATUS[*]}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "pipe=0 1\n");
    assert_eq!(stderr, div0(&path, 1));
}

#[test]
fn issue3456_prefix_arith_wc_second_element() {
    let (path, stdout, stderr, code) = run_script(
        "q8-wc",
        "printf \"a\\nb\\n\" | x=$((1/0)) wc -l\necho \"pipe=${PIPESTATUS[*]}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "pipe=0 1\n");
    assert_eq!(stderr, div0(&path, 1));
}

#[test]
fn issue3456_prefix_arith_tr_second_element() {
    let (path, stdout, stderr, code) = run_script(
        "q9-tr",
        "printf \"a\\n\" | x=$((1/0)) tr a b\necho \"pipe=${PIPESTATUS[*]}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "pipe=0 1\n");
    assert_eq!(stderr, div0(&path, 1));
}

#[test]
fn issue3456_prefix_arith_head_second_element() {
    let (path, stdout, stderr, code) = run_script(
        "q10-head",
        "printf \"a\\nb\\n\" | x=$((1/0)) head -1\necho \"pipe=${PIPESTATUS[*]}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "pipe=0 1\n");
    assert_eq!(stderr, div0(&path, 1));
}

#[test]
fn issue3456_prefix_arith_trap_first_element() {
    let (path, stdout, stderr, code) = run_script(
        "q11-trap",
        "x=$((1/0)) trap -p | cat\necho \"pipe=${PIPESTATUS[*]}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "pipe=1 0\n");
    assert_eq!(stderr, div0(&path, 1));
}

#[test]
fn issue3456_prefix_arith_printenv_never_spawns() {
    // Face B: the diagnostic and the status were already right; the
    // child still spawned and printed an empty line. GNU spawns
    // nothing, so the only stdout is the pipe-status line.
    let (path, stdout, stderr, code) = run_script(
        "r1-printenv",
        "x=$((1/0)) printenv x | cat\necho \"pipe=${PIPESTATUS[*]}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "pipe=1 0\n");
    assert_eq!(stderr, div0(&path, 1));
}

#[test]
fn issue3456_prefix_arith_sh_never_spawns() {
    let (path, stdout, stderr, code) = run_script(
        "r2-sh",
        "x=$((1/0)) sh -c \"echo in-child x=[$x]\" | cat\necho \"pipe=${PIPESTATUS[*]}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "pipe=1 0\n");
    assert_eq!(stderr, div0(&path, 1));
}

#[test]
fn issue3456_prefix_arith_wc_file_never_spawns() {
    let (path, stdout, stderr, code) = run_script(
        "r3-wcfile",
        "x=$((1/0)) wc -l /dev/null | cat\necho \"pipe=${PIPESTATUS[*]}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "pipe=1 0\n");
    assert_eq!(stderr, div0(&path, 1));
}

#[test]
fn issue3456_prefix_arith_yes_streaming_tail_never_spawns() {
    // The streaming tail (`yes | head -1`) is a concurrent path, not
    // the sequential one — GNU still discards the `yes` element.
    let (path, stdout, stderr, code) = run_script(
        "r4-yes",
        "x=$((1/0)) yes | head -1\necho \"pipe=${PIPESTATUS[*]}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "pipe=1 0\n");
    assert_eq!(stderr, div0(&path, 1));
}

#[test]
fn issue3456_prefix_arith_function_stage_guard() {
    // Already green on the base head; guard against regressions.
    let (path, stdout, stderr, code) = run_script(
        "s1-func",
        "f() { echo hi; }\nx=$((1/0)) f | cat\necho \"pipe=${PIPESTATUS[*]}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "pipe=1 0\n");
    assert_eq!(stderr, div0(&path, 2));
}

#[test]
fn issue3456_prefix_arith_pipefail() {
    let (path, stdout, stderr, code) = run_script(
        "s2-pipefail",
        "set -o pipefail\nx=$((1/0)) echo hi | cat\necho \"rc=$? pipe=${PIPESTATUS[*]}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1 pipe=1 0\n");
    assert_eq!(stderr, div0(&path, 2));
}

#[test]
fn issue3456_prefix_arith_background_pipeline() {
    let (path, stdout, stderr, code) =
        run_script("s3-bg", "x=$((1/0)) echo hi | cat &\nwait\necho done\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "done\n");
    assert_eq!(stderr, div0(&path, 1));
}

#[test]
fn issue3456_prefix_arith_command_substitution_guard() {
    // Already green on the base head; guard against regressions. The
    // comsub's status is the inner command's status, so rc=1.
    let (path, stdout, stderr, code) = run_script(
        "s4-comsub",
        "x=$(x=$((1/0)) echo hi)\necho \"got=[$x] rc=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "got=[] rc=1\n");
    assert_eq!(stderr, div0(&path, 1));
}

#[test]
fn issue3456_prefix_arith_nounset_first_element() {
    // GNU: `undef: unbound variable`, element status 1 (not 127).
    let (path, stdout, stderr, code) = run_script(
        "s5-nounset",
        "set -u\nx=$((undef)) echo hi | cat\necho \"pipe=${PIPESTATUS[*]}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "pipe=1 0\n");
    assert_eq!(stderr, format!("{path}: line 2: undef: unbound variable\n"));
}

#[test]
fn issue3456_prefix_arith_nounset_second_element() {
    let (path, stdout, stderr, code) = run_script(
        "n4-nounset2",
        "set -u\necho hi | x=$((undef)) cat\necho \"pipe=${PIPESTATUS[*]}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "pipe=0 1\n");
    assert_eq!(stderr, format!("{path}: line 2: undef: unbound variable\n"));
}

#[test]
fn issue3456_prefix_arith_assign_to_nonvariable() {
    let (path, stdout, stderr, code) = run_script(
        "s6-eq",
        "x=$((1=2)) echo hi | cat\necho \"pipe=${PIPESTATUS[*]}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "pipe=1 0\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: 1=2: attempted assignment to non-variable (error token is \"=2\")\n"
        )
    );
}

#[test]
fn issue3456_prefix_arith_invalid_operator() {
    // Rubash prints `arithmetic syntax error` where GNU prints `syntax
    // error` for this shape — a pre-existing evaluator wording
    // divergence, pinned as rubash prints it.
    let (path, stdout, stderr, code) = run_script(
        "s7-sub",
        "x=$((1[0]=2)) echo hi | cat\necho \"pipe=${PIPESTATUS[*]}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "pipe=1 0\n");
    assert_eq!(
        stderr,
        format!("{path}: line 1: 1[0]=2: arithmetic syntax error: invalid arithmetic operator (error token is \"[0]=2\")\n")
    );
}

#[test]
fn issue3456_prefix_arith_octal_too_great() {
    let (path, stdout, stderr, code) = run_script(
        "s8-oct",
        "x=$((08)) echo hi | cat\necho \"pipe=${PIPESTATUS[*]}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "pipe=1 0\n");
    assert_eq!(
        stderr,
        format!("{path}: line 1: 08: value too great for base (error token is \"08\")\n")
    );
}

#[test]
fn issue3456_prefix_comsub_side_effect_runs() {
    // GNU expands the prefix RHS in the forked child, so the comsub
    // side effect happens even though the element itself is discarded
    // by nothing here (the prefix is clean, the element runs). The
    // probe that fixes face A expands the RHS once, so the file is
    // created — on the base head the inline arms never expanded the
    // RHS at all and the file was missing.
    let (path, stdout, stderr, code) = run_script(
        "s9-side",
        "rm -f side.txt\nx=$(echo SIDE > side.txt) echo hi | cat\necho \"pipe=${PIPESTATUS[*]}\"\ntest -f side.txt && echo created || echo missing\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "hi\npipe=0 0\ncreated\n");
    assert_eq!(stderr, "");
    let _ = path;
}

#[test]
fn issue3456_prefix_incr_rolls_back_guard() {
    // The prefix's own write runs in the forked child on GNU, so the
    // parent never sees `y`; the probe's writes must roll back too.
    let (_path, stdout, stderr, code) = run_script(
        "s10-incr",
        "x=$((y+=1)) echo hi | cat\necho \"pipe=${PIPESTATUS[*]} y=[$y]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "hi\npipe=0 0 y=[]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue3456_prefix_incr_word_expands_first_guard() {
    // Words expand before the prefix binds (GNU prints 5, `i` stays
    // 5); the probe must not disturb that order.
    let (_path, stdout, stderr, code) = run_script(
        "s11-i",
        "i=5\nx=$((i++)) echo $((i)) | cat\necho \"pipe=${PIPESTATUS[*]} i=$i\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "5\npipe=0 0 i=5\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue3456_prefix_hello_word_expands_first_delimitation() {
    // Delimitation: `x=hello echo "stage x=[$x]"` prints `stage x=[]`
    // on GNU too — the word is expanded against the outer environment.
    let (_path, stdout, stderr, code) = run_script(
        "s12-hello",
        "x=hello echo \"stage x=[$x]\"\necho \"pipe=${PIPESTATUS[*]}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "stage x=[]\npipe=0\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue3456_prefix_before_compound_is_syntax_error_delimitation() {
    // Delimitation: a prefix before a compound command is a syntax
    // error on GNU too (rc 2), so there is nothing to fix here.
    let (path, stdout, stderr, code) =
        run_script("s13-compound", "x=1 { echo hi; }\necho \"rc=$?\"\n");
    assert_eq!(code, Some(2));
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: syntax error near unexpected token `}}'\n{path}: line 1: `x=1 {{ echo hi; }}'\n"
        )
    );
}

#[test]
fn issue3456_prefix_nounset_plain_word_guard() {
    // Already green on the base head; GNU status is 1 here too.
    let (path, stdout, stderr, code) = run_script(
        "n2-dollar",
        "set -u\nx=$undef echo hi | cat\necho \"pipe=${PIPESTATUS[*]}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "pipe=1 0\n");
    assert_eq!(stderr, format!("{path}: line 2: undef: unbound variable\n"));
}

#[test]
fn issue3456_prefix_nounset_word_guard() {
    // Already green on the base head; GNU status is 1 here too.
    let (path, stdout, stderr, code) = run_script(
        "n6-word",
        "set -u\necho $undef | cat\necho \"pipe=${PIPESTATUS[*]}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "pipe=1 0\n");
    assert_eq!(stderr, format!("{path}: line 2: undef: unbound variable\n"));
}
