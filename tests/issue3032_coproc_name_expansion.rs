//! rubash#3032: `coproc $namevar { foo; }` with `$namevar` unset
//! reported the UNEXPANDED name (`` `$namevar': not a valid
//! identifier ``); GNU reports `(null)`.
//!
//! GNU ground truth (bash 5.3.20, verified by script-file probes):
//! execute_coproc (execute_cmd.c:2503-2511) FIRST expands the name
//! without splitting — `expand_string_unsplit_to_string
//! (command->value.Coproc->name, 0)` — and only then validates:
//! `if (valid_identifier (name) == 0) { err_invalidid (name);
//! return EXECUTION_FAILURE; }`, spawning nothing. When the
//! expansion comes out empty (unset or empty value) the expansion
//! returns NULL, and err_invalidid (error.c:459-462,
//! `internal_error ("\`%s': not a valid identifier", s)`) renders
//! the NULL %s as `(null)`. A quoted empty name (`coproc ""`)
//! expands to a real empty string instead, and reports `` `': ``.
//!
//! Rubash validated the parse-time literal word: the unset case
//! misreported `$namevar`, an invalid VALUE was never shown, and a
//! valid expanded name was rejected outright.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_driver(tag: &str, files: &[(&str, &str)], driver: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-3032-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    for (name, body) in files {
        let mut file = std::fs::File::create(dir.join(name)).expect("write fixture");
        file.write_all(body.as_bytes()).expect("write fixture");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(driver)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The issue seed: unset name variable expands to NULL, reported
/// as `(null)`, the coproc attempt fails with rc 1, nothing runs.
#[test]
fn coproc_unset_name_var_reports_null() {
    let (stdout, stderr, rc) = run_driver(
        "unset",
        &[("drv.sh", "coproc $nv { echo hi; }\necho \"rc=$?\"\n")],
        "drv.sh",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(
        stderr, "drv.sh: line 1: `(null)': not a valid identifier\n",
        "GNU expands the name first; an unset variable expands to NULL"
    );
}

/// A set-but-empty name variable takes the same NULL path.
#[test]
fn coproc_empty_name_var_reports_null() {
    let (stdout, stderr, rc) = run_driver(
        "empty",
        &[("drv.sh", "nv=\ncoproc $nv { echo hi; }\necho \"rc=$?\"\n")],
        "drv.sh",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(
        stderr, "drv.sh: line 2: `(null)': not a valid identifier\n",
        "an empty expansion is NULL to err_invalidid, like the unset case"
    );
}

/// An invalid EXPANDED value is what the message names — not the
/// literal `$nv` word.
#[test]
fn coproc_invalid_expanded_name_reports_expanded_value() {
    let (stdout, stderr, rc) = run_driver(
        "badval",
        &[(
            "drv.sh",
            "nv=9bad\ncoproc $nv { echo hi; }\necho \"rc=$?\"\n",
        )],
        "drv.sh",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(
        stderr, "drv.sh: line 2: `9bad': not a valid identifier\n",
        "the diagnostic names the expanded value"
    );
}

/// Chain: a valid expanded name creates the coproc under that
/// name — the fd array and the _PID variable land under the
/// EXPANDED name, and the coprocess's output reads back.
#[test]
fn coproc_valid_expanded_name_creates_named_coproc() {
    let (stdout, stderr, rc) = run_driver(
        "valid",
        &[(
            "drv.sh",
            "nv=MYCP\ncoproc $nv { echo hi; }\nread -u ${MYCP[0]} line\necho \"got=$line pid=${MYCP_PID:+set}\"\n",
        )],
        "drv.sh",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "got=hi pid=set\n");
}

/// Control: a literal invalid name reports the literal (this
/// shape already agreed with GNU).
#[test]
fn coproc_literal_invalid_name_reports_literal() {
    let (stdout, stderr, rc) = run_driver(
        "literal",
        &[("drv.sh", "coproc 9bad { echo hi; }\necho \"rc=$?\"\n")],
        "drv.sh",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(stderr, "drv.sh: line 1: `9bad': not a valid identifier\n");
}

/// Control: a quoted empty name expands to a real empty string,
/// reported as `` `': `` — NOT `(null)` (rubash#1153 shape).
#[test]
fn coproc_quoted_empty_name_reports_empty() {
    let (stdout, stderr, rc) = run_driver(
        "quoted",
        &[("drv.sh", "coproc \"\" { echo hi; }\necho \"rc=$?\"\n")],
        "drv.sh",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(stderr, "drv.sh: line 1: `': not a valid identifier\n");
}
