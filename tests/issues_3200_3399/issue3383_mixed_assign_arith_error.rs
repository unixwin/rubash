//! Regression pins for rubash#3383 (mixed-assignment face only).
//!
//! Issue #3383 has two faces: (1) a mixed/compound assignment whose
//! element expansion fails fatally leaves the target unbound, and
//! (2) a pipeline element's prefix assignment ignores the same error
//! entirely (no diagnostic, stage runs, status 0). This stem pins
//! face (1), fixed by this PR; face (2) is pinned by the probe set
//! in the lane workdir (`probes/`, GNU results on disk) and fixed in
//! follow-up — its cells are deliberately NOT in this stem, because
//! they are still red on this head.
//!
//! Mechanism (face 1): GNU arrayfunc.c:507-517 assign_array_from_string
//! runs find_or_make_array_variable BEFORE assign_array_var_from_string
//! expands the elements, so a brand-new compound target is bound as an
//! empty indexed array even when an element's expansion fails fatally
//! and the command is abandoned (DISCARD, status 1). Rubash's
//! execute_empty_words_command (command_prepare.rs) already kept this
//! contract in its NONFATAL arm but returned from the FATAL arm without
//! binding, so `a=($((1/0)) 2)` left `a` unbound (`declare: a: not
//! found`) where GNU leaves `declare -a a=()`. The fix binds the empty
//! array in the fatal arm too, through the shared helper
//! bind_empty_array_for_failed_compound_assignment (also used by the
//! nonfatal arm now, so the two arms cannot drift apart again).
//!
//! Delimitations, all GNU-oracle-verified (bash 5.3.20):
//!   - a PRE-EXISTING target keeps its prior elements: `a=(1 2)` then
//!     `a+=($((1/0)))` leaves `a=([0]="1" [1]="2")` — the helper is a
//!     no-op when the base name already exists (niubash #121).
//!   - a trailing element after the failure never binds: `a+=( $((1/0))
//!     3)` still leaves `a=([0]="1" [1]="2")`.
//!   - a `declare -a a=(...)` failure is a DIFFERENT mechanism: GNU
//!     leaves `a` unbound there (the declare builtin binds only after
//!     its own expansion succeeds), so the declare path is out of scope
//!     for this fix and has no cell here.
//!   - the failing assignment still abandons its command list: inside a
//!     function body the remaining lines never run, but the empty array
//!     IS bound and visible to the caller afterwards.
//!
//! GNU expectations below are the literal GNU bash 5.3.20 outputs for
//! these scripts (oracle binary
//! ~/workspace/rubash-audit/gnu-bash-5.3.20/bin/bash, LC_ALL=C.UTF-8).
#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3383-{tag}-{}", std::process::id()));
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

#[test]
fn issue3383_array_compound_fatal_first_element() {
    // The issue's m8 shape: the first element fails, the array must
    // still exist as an empty indexed array afterwards.
    let (path, stdout, stderr, code) = run_script(
        "m8-first",
        "a=($((1/0)) 2)\necho \"after rc=$?\"\ndeclare -p a 2>&1\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after rc=1\ndeclare -a a=()\n");
    assert_eq!(
        stderr,
        format!("{path}: line 1: 1/0: division by 0 (error token is \"0\")\n")
    );
}

#[test]
fn issue3383_array_single_fatal_element() {
    // m8b: a single failing element — same empty-array contract.
    let (path, stdout, stderr, code) = run_script(
        "m8b-single",
        "a=($((1/0)))\necho \"after rc=$?\"\ndeclare -p a 2>&1\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after rc=1\ndeclare -a a=()\n");
    assert_eq!(
        stderr,
        format!("{path}: line 1: 1/0: division by 0 (error token is \"0\")\n")
    );
}

#[test]
fn issue3383_array_fatal_middle_element() {
    // m8c: the failure is in the middle — the already-expanded `1`
    // must NOT survive: the array is empty, not partial. GNU creates
    // the variable before expanding ANY element (arrayfunc.c:507-517),
    // and the expansion failure abandons the whole assignment.
    let (path, stdout, stderr, code) = run_script(
        "m8c-middle",
        "a=(1 $((1/0)) 3)\necho \"after rc=$?\"\ndeclare -p a 2>&1\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after rc=1\ndeclare -a a=()\n");
    assert_eq!(
        stderr,
        format!("{path}: line 1: 1/0: division by 0 (error token is \"0\")\n")
    );
}

#[test]
fn issue3383_array_append_fatal_keeps_prior() {
    // m8d: `+=` onto an existing array — the prior elements survive
    // (the helper is a no-op when the base already exists).
    let (path, stdout, stderr, code) = run_script(
        "m8d-append",
        "a=(1 2)\na+=($((1/0)))\necho \"after rc=$?\"\ndeclare -p a 2>&1\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after rc=1\ndeclare -a a=([0]=\"1\" [1]=\"2\")\n");
    assert_eq!(
        stderr,
        format!("{path}: line 2: 1/0: division by 0 (error token is \"0\")\n")
    );
}

#[test]
fn issue3383_array_fatal_second_element() {
    // m8f: the failure is the second element, after a plain first one.
    let (path, stdout, stderr, code) = run_script(
        "m8f-second",
        "a=(x $((1/0)))\necho \"after rc=$?\"\ndeclare -p a 2>&1\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after rc=1\ndeclare -a a=()\n");
    assert_eq!(
        stderr,
        format!("{path}: line 1: 1/0: division by 0 (error token is \"0\")\n")
    );
}

#[test]
fn issue3383_array_append_fatal_trailing_discarded() {
    // A trailing element after the failure never binds either.
    let (path, stdout, stderr, code) = run_script(
        "append-trailing",
        "a=(1 2)\na+=($((1/0)) 3)\necho \"after rc=$?\"\ndeclare -p a 2>&1\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after rc=1\ndeclare -a a=([0]=\"1\" [1]=\"2\")\n");
    assert_eq!(
        stderr,
        format!("{path}: line 2: 1/0: division by 0 (error token is \"0\")\n")
    );
}

#[test]
fn issue3383_array_fatal_in_function_binds_empty() {
    // The failing assignment abandons the rest of the function body
    // (the body's own `echo` never runs — DISCARD), but the empty
    // array IS bound and visible to the caller afterwards.
    let (path, stdout, stderr, code) = run_script(
        "func-body",
        "f(){ a=($((1/0)) 2); echo \"after rc=$?\"; }\nf\necho \"outer rc=$?\"\ndeclare -p a 2>&1\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "outer rc=1\ndeclare -a a=()\n");
    assert_eq!(
        stderr,
        format!("{path}: line 1: 1/0: division by 0 (error token is \"0\")\n")
    );
}

#[test]
fn issue3383_array_fatal_in_eval_binds_empty() {
    // The same shape through eval — the eval'd assignment runs the
    // same empty-words path.
    let (path, stdout, stderr, code) = run_script(
        "eval-body",
        "eval 'a=($((1/0)) 2)'\necho \"after rc=$?\"\ndeclare -p a 2>&1\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after rc=1\ndeclare -a a=()\n");
    assert_eq!(
        stderr,
        format!("{path}: line 1: 1/0: division by 0 (error token is \"0\")\n")
    );
}

#[test]
fn issue3383_control_prefix_simple_fatal() {
    // Control (c1): a fatal prefix assignment before a plain command
    // already worked — diagnostic once, command skipped, status 1.
    let (path, stdout, stderr, code) =
        run_script("c1-control", "x=$((1/0)) echo hi\necho \"after rc=$?\"\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after rc=1\n");
    assert_eq!(
        stderr,
        format!("{path}: line 1: 1/0: division by 0 (error token is \"0\")\n")
    );
}

#[test]
fn issue3383_control_bare_assign_fatal() {
    // Control (c2): a bare fatal assignment leaves the scalar empty
    // and the script continues with status 1 on that line.
    let (path, stdout, stderr, code) =
        run_script("c2-control", "x=$((1/0))\necho \"after rc=$? x=[$x]\"\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after rc=1 x=[]\n");
    assert_eq!(
        stderr,
        format!("{path}: line 1: 1/0: division by 0 (error token is \"0\")\n")
    );
}

#[test]
fn issue3383_control_bare_assign_then_arith() {
    // Control (c3, the #3287 canary shape): after a fatal bare
    // assignment, the next `(( ))` evaluates normally — no flag leak.
    let (path, stdout, stderr, code) = run_script(
        "c3-control",
        "x=$((1/0))\n(( y = 2 ))\necho \"after rc=$? y=[$y]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after rc=0 y=[2]\n");
    assert_eq!(
        stderr,
        format!("{path}: line 1: 1/0: division by 0 (error token is \"0\")\n")
    );
}
