//! Parse / diagnostic-wording regression pins (backfill wave bf-c).
//!
//! rubash#1064 — `break` / `continue` used as a pipeline element printed
//! a spurious `only meaningful in a loop` diagnostic; GNU runs them
//! silently (the loop is unaffected, stdout/rc already agreed).
//! rubash#1066 — `break 1 2` / `continue 1 2` (too many arguments) was
//! non-fatal: GNU reports `too many arguments` ONCE and aborts the
//! enclosing loop with status 2; in script files execution continues
//! with the next line, in a `-c` string the shell exits.
//! rubash#1085 — `cd` invalid-option usage string omitted `[-@]`; GNU:
//! `cd: usage: cd [-L|[-P [-e]]] [-@] [dir]`, rc 2.
//! rubash#1310 — a function definition with an EMPTY subshell body
//! (`f() ( )`) was accepted; GNU rejects it at parse time (rc 2,
//! `syntax error near unexpected token ')'`), in all three spellings.
//!
//! Expectations hand-measured against GNU bash 5.3.20 via script files
//! and `-c` probes in this environment (byte-level stdout/stderr/rc).
//! Note on #1066 `-c` exit status: the reporter's Termux GNU build
//! exited 2; this environment's GNU bash 5.3.20 exits 1 for the `-c`
//! form while the script-file form observes status 2 via `$?`. Both
//! forms are pinned as measured here.
//!
//! DRIFT (found by backfill bf-c, 2026-10-25; FIXED by rubash#1519):
//! master made the too-many-arguments error fatal to the WHOLE script
//! (exit 2, no tail line runs) in BOTH forms. The four #1066 pins below
//! are live again: abort the enclosing loop with status 2 and continue
//! (script file), exit 1 (`-c` string).

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir =
        std::env::temp_dir().join(format!("rubash-bfc-parse-{}-{tag}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn run_c(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn run_script(script: &str) -> (String, String, Option<i32>) {
    let dir = scratch("script");
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(path.to_str().expect("utf8 path"))
        .output()
        .expect("run rubash");
    let out = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    out
}

#[test]
fn issue_1064_break_continue_as_pipeline_element_are_silent() {
    let (stdout, stderr, code) = run_script(
        "for i in 1 2; do break | cat; echo \"after $i\"; done\nfor i in 1 2; do continue | cat; echo \"cafter $i\"; done\necho \"rc=$?\"\n",
    );
    assert_eq!(
        stdout, "after 1\nafter 2\ncafter 1\ncafter 2\nrc=0\n",
        "stderr: {stderr}"
    );
    assert_eq!(stderr, "", "no spurious loop diagnostic");
    assert_eq!(code, Some(0));
}

#[test]
fn issue_1066_break_too_many_args_aborts_loop_with_status_2() {
    let (stdout, stderr, code) =
        run_script("for i in 1 2; do break 1 2; echo \"hi $i\"; done\necho \"rc=$?\"\n");
    // The loop dies at the error (no `hi` lines); the NEXT script line
    // observes status 2.
    assert_eq!(stdout, "rc=2\n", "stderr: {stderr}");
    assert_eq!(
        stderr.matches("break: too many arguments").count(),
        1,
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue_1066_continue_too_many_args_aborts_loop_with_status_2() {
    let (stdout, stderr, code) =
        run_script("for i in 1 2; do continue 1 2; echo \"hi $i\"; done\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=2\n", "stderr: {stderr}");
    assert_eq!(
        stderr.matches("continue: too many arguments").count(),
        1,
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue_1066_nested_break_too_many_args_aborts_outer_loop() {
    let (stdout, stderr, code) = run_script(
        "for i in 1 2; do for j in 1 2; do break 2 3; echo \"inner $i $j\"; done; echo \"outer $i\"; done\necho end\n",
    );
    assert_eq!(stdout, "end\n", "stderr: {stderr}");
    assert_eq!(
        stderr.matches("break: too many arguments").count(),
        1,
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue_1066_dash_c_form_exits_without_running_the_tail() {
    let (stdout, stderr, code) =
        run_c("for i in 1 2; do break 1 2; echo \"hi $i\"; done; echo \"rc=$?\"");
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert_eq!(
        stderr.matches("break: too many arguments").count(),
        1,
        "stderr: {stderr}"
    );
    // GNU bash 5.3.20 (this environment) exits 1 here; see header note.
    assert_eq!(code, Some(1));
}

#[test]
fn issue_1085_cd_usage_string_includes_at_option() {
    let (stdout, stderr, code) = run_c("cd -Z /tmp");
    assert_eq!(stdout, "");
    assert!(stderr.contains("invalid option"), "stderr: {stderr}");
    assert!(
        stderr.contains("cd: usage: cd [-L|[-P [-e]]] [-@] [dir]"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue_1310_empty_subshell_function_body_is_a_syntax_error() {
    for form in [
        "f() ( ); echo \"rc=$?\"",
        "f() (); echo \"rc=$?\"",
        "function f() ( ); echo \"rc=$?\"",
    ] {
        let (stdout, stderr, code) = run_c(form);
        assert_eq!(stdout, "", "form {form}");
        assert!(
            stderr.contains("syntax error near unexpected token"),
            "form {form} stderr: {stderr}"
        );
        assert_eq!(code, Some(2), "form {form}");
    }
}

#[test]
fn issue_1310_nonempty_subshell_body_and_bare_empty_subshell() {
    // Control: a non-empty subshell body defines and runs fine.
    let (stdout, _stderr, code) = run_c("f() ( echo hi ); f");
    assert_eq!(stdout, "hi\n");
    assert_eq!(code, Some(0));
    // Control: the bare empty subshell is rejected by both shells.
    let (stdout, _stderr, code) = run_c("( ); echo rc=$?");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(2));
}
