//! unixwin/rubash#705 R7.1: the nounset fatal exit status under `-e`.
//!
//! GNU's fatal word-expansion error (`${var?}` / `set -u` / nounset
//! arithmetic) is reported through error.c:169 `report_error`, which, when
//! `exit_immediately_on_error` is set (flags.c:263, the `-e` / `set -e`
//! option), calls `exit_shell(last_command_exit_value)` — an EXITPROG jump
//! with status 1 (EXECUTION_FAILURE). Only with errexit OFF does the jump
//! stay FORCE_EOF and reach shell.c:1471 run_one_command's remap to 127.
//! So `bash -eu -c 'echo $UNDEF'` exits 1 while `bash -u -c 'echo $UNDEF'`
//! exits 127. The predicate is the option VALUE at the error site: an
//! `if`/`||` context does not clear it (GNU 5.3.0 oracle, 2026-10-17).
//!
//! Oracle: WSL GNU bash 5.3.0(1)-release, file-based comparisons; every
//! expected status below was read from that shell. The CWD is a private
//! sandbox so no ambient `./nounset.sh` can be picked up.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn sandbox(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-705-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create sandbox dir");
    dir
}

fn run(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(args)
        .current_dir(dir)
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .env("HOME", dir)
        .output()
        .expect("spawn rubash");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// `-eu` and `-u` must fork exactly as GNU bash does: errexit turns the
/// fatal nounset exit from 127 into 1, and only errexit does.
#[test]
fn nounset_fatal_status_accounts_for_errexit() {
    let dir = sandbox("nounset-errexit");
    let cases: &[(&[&str], i32)] = &[
        // Adding -e flips 127 -> 1 (the reported divergence).
        (&["-eu", "-c", "echo $UNDEF"], 1),
        (&["-u", "-c", "echo $UNDEF"], 127),
        // The option can be toggled at runtime; the value at the error site wins.
        (&["-u", "-c", "set -e; echo $UNDEF"], 1),
        (&["-eu", "-c", "set +e; echo $UNDEF"], 127),
        // `||` does not clear exit_immediately_on_error, so the fatal still
        // exits 1 (GNU 5.3.0 oracle; the FORCE_EOF/DISCARD split does not
        // apply to the report_error branch).
        (&["-eu", "-c", "false || echo $UNDEF"], 1),
        // `${var?}` and nounset arithmetic share the fatal status owner.
        (&["-eu", "-c", "echo ${X?}"], 1),
        (&["-u", "-c", "echo ${X?}"], 127),
        (&["-eu", "-c", "echo $((NOPE))"], 1),
        (&["-u", "-c", "echo $((NOPE))"], 127),
    ];
    for (args, expected) in cases {
        let (rc, _out, _err) = run(&dir, args);
        assert_eq!(rc, *expected, "rubash {:?}", args);
    }
}

/// The fatal diagnostic itself stays byte-identical to GNU bash; only the
/// status changes with errexit.
#[test]
fn nounset_diagnostic_is_gnu_bash_prefixed() {
    let dir = sandbox("nounset-bytes");
    let (rc, stdout, stderr) = run(&dir, &["-eu", "-c", "echo $UNDEF"]);
    assert_eq!(rc, 1);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "bash: line 1: UNDEF: unbound variable\n");
}

/// In script mode the FORCE_EOF reader (eval.c:98-107) already exits with
/// last_command_exit_value = 1 regardless of errexit; `-e` must not disturb
/// that.
#[test]
fn nounset_fatal_status_in_script_mode_is_one() {
    let dir = sandbox("nounset-script");
    fs::write(dir.join("nounset.sh"), "echo $UNDEF\n").expect("write script");
    for args in [&["-u", "nounset.sh"][..], &["-eu", "nounset.sh"][..]] {
        let (rc, _out, err) = run(&dir, args);
        assert_eq!(rc, 1, "rubash {args:?}");
        assert_eq!(err, "nounset.sh: line 1: UNDEF: unbound variable\n");
    }
}

/// A forked child (`( ... )`) contains the jump and exits 1 even without
/// -e; errexit must not regress that either.
#[test]
fn nounset_status_inside_a_subshell_child_is_one() {
    let dir = sandbox("nounset-subshell");
    let (rc, _out, _err) = run(&dir, &["-eu", "-c", "( echo $UNDEF )"]);
    assert_eq!(rc, 1);
}
