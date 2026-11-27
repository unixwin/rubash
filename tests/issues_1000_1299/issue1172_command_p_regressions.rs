//! rubash#1172 — command -p missingbin: niu fails exec rc=126, GNU
//! fails lookup rc=127.
//!
//! GNU builtins/command.def: `command -p` looks the name up in the
//! DEFAULT path; a name that is not there is a lookup failure —
//! "command not found", rc=127 — not an exec failure (126). A name
//! that IS in the default path runs normally, and `command -p -v`
//! prints its default-path location.
//!
//! Expectations verified against GNU bash 5.3.20 (`-c` strings) on
//! this host (the default path value and getconf location are the
//! host's confstr defaults, identical for both shells).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn command_p_missing_name_is_lookup_failure_127() {
    let (stdout, stderr, code) = rubash(
        "PATH=/nonexistent_xyz; command -p getconf PATH; echo rc=$?\n\
         PATH=/nonexistent_xyz; command -p missingbin_xyz; echo rc=$?",
    );
    assert_eq!(stdout, "/bin:/usr/bin\nrc=0\nrc=127\n");
    assert!(
        stderr.contains("missingbin_xyz: command not found"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn command_p_v_and_bare_forms() {
    let (stdout, _, code) =
        rubash("command -p -v getconf; echo rc=$?\ncommand -p getconf; echo rc=$?");
    assert_eq!(stdout, "/bin/getconf\nrc=0\nrc=2\n");
    assert_eq!(code, Some(0));
}

#[test]
fn command_p_unknown_names_all_127() {
    let (stdout, stderr, code) = rubash(
        "command -p nxutil_x; echo rc=$?\n\
         command -p biutil_x; echo rc=$?\n\
         command -p crutil_x; echo rc=$?\n\
         nxutil_x; echo rc=$?\n\
         biutil_x; echo rc=$?\n\
         crutil_x; echo rc=$?",
    );
    assert_eq!(stdout, "rc=127\nrc=127\nrc=127\nrc=127\nrc=127\nrc=127\n");
    for name in ["nxutil_x", "biutil_x", "crutil_x"] {
        assert!(
            stderr
                .matches(&format!("{name}: command not found"))
                .count()
                == 2,
            "stderr: {stderr}"
        );
    }
    assert_eq!(code, Some(0));
}
