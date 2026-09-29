//! Issue rubash#331 regression (approved option A from #329,
//! docs/issue329-path-form-policy-analysis.md): rubash's OWN produced path
//! values render in POSIX form (/d/...) like `$PWD`:
//! - `$0` from a full-path argv[0] (`bash -c` shape) was backslash Windows
//!   form (#224 legacy);
//! - `BASH` was forward-slash drive form (D:/...);
//! - the self-injected TMPDIR default (no TMPDIR/TEMP/TMP in the parent
//!   env) was backslash Windows form.
//! Inherited values keep their verbatim import (GNU semantics); only
//! self-produced values are normalized.

use std::process::Command;

fn run(args: &[&str], env_clear: bool) -> (String, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command.args(args);
    if env_clear {
        command
            .env_remove("TMPDIR")
            .env_remove("TEMP")
            .env_remove("TMP");
    }
    let output = command.output().expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// `$0` in `-c` mode (argv[0]-derived) is POSIX: leading slash, no
/// backslashes, no drive-letter colon.
#[test]
fn c_mode_zero_is_posix_form() {
    let (stdout, stderr) = run(&["-c", "echo $0"], false);
    let zero = stdout.trim_end();
    assert!(
        zero.starts_with('/') && !zero.contains('\\') && !zero.contains(':'),
        "$0 was {zero:?}, stderr: {stderr}"
    );
}

/// `BASH` renders in the same POSIX form as `$PWD`.
#[test]
fn bash_variable_is_posix_form() {
    let (stdout, stderr) = run(&["-c", "echo $BASH"], false);
    let bash = stdout.trim_end();
    assert!(
        bash.starts_with('/') && !bash.contains('\\') && !bash.contains(':'),
        "BASH was {bash:?}, stderr: {stderr}"
    );
}

/// `$PWD` and `$(pwd)` agree in form and value in a /d/ cwd.
#[test]
fn pwd_and_pwd_substitution_agree() {
    let (stdout, stderr) = run(
        &["-c", "[[ $PWD == $(pwd) ]] && echo SAME || echo DIFF"],
        false,
    );
    assert_eq!(stdout.trim_end(), "SAME", "stderr: {stderr}");
}

/// The self-injected TMPDIR default (parent env has no TMPDIR/TEMP/TMP) is
/// POSIX form — the FFmpeg-configure corollary: backslash or drive forms
/// corrupt scripts that interpolate `$TMPDIR` unquoted.
#[test]
fn injected_tmpdir_default_is_posix_form() {
    let (stdout, stderr) = run(&["-c", "echo $TMPDIR"], true);
    let tmpdir = stdout.trim_end();
    assert!(
        tmpdir.starts_with('/') && !tmpdir.contains('\\') && !tmpdir.contains(':'),
        "TMPDIR was {tmpdir:?}, stderr: {stderr}"
    );
}
