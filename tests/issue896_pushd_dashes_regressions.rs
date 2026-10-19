//! Issue rubash#896 regression: `pushd -- -n` must fail like `cd` (rc 1,
//! `pushd: -n: No such file or directory`), not a usage error (rc 2) — a
//! leading `--` sets skipopt and skips the whole option walk
//! (pushd.def:183-190). Same family: a lone `-` is `cd $OLDPWD`, options
//! with no operand left are a silent no-op, and a multi-word Dir operand is
//! cd's `too many arguments` (rc 1, no usage line).
//!
//! GNU spec (third_party/bash, Bash-5.3 patch 15):
//! pushd.def:183-190 leading `--` (skipopt); :214-260 option walk (`-n`,
//! `--`, lone `-`); :288 `list == 0` silent success; :298 cd_builtin
//! delegation; cd.def:326-330 CD_COMPLAINS argc check (active via
//! config-top.h); cd.def:339-349 `cd -` OLDPWD arm + :371-374 LCD_PRINTPATH.
//! All verdicts below were probed byte-identical against WSL GNU Bash 5.3.0
//! script files before the fix.
//!
//! ```text
//! $ pushd -- -n    # 1  pushd: -n: No such file or directory (no usage)
//! $ pushd -        # 0  echoes $OLDPWD, then the stack (OLDPWD=/tmp)
//! $ pushd -n       # 0  silent no-op (rubash swapped the stack)
//! $ pushd -n --    # 0  silent no-op
//! $ pushd / extra  # 1  pushd: too many arguments (no usage)
//! $ pushd - -n     # 1  pushd: too many arguments
//! $ pushd -n -- +1 # 0  pushes a literal `+1`, never rotates
//! ```

#![cfg(windows)]

use std::io::Write;
use std::process::{Command, Stdio};

fn run(script: &str) -> (Option<i32>, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(script.as_bytes())
        .expect("write script to stdin");
    let output = child.wait_with_output().expect("wait for rubash");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The ticket case: `-- -n` is a cd failure, not a usage error.
#[test]
fn pushd_double_dash_dash_n_is_cd_failure_not_usage() {
    let (code, stdout, stderr) = run("pushd -- -n\n");
    assert_eq!(code, Some(1), "rc for `pushd -- -n`: {stderr:?}");
    assert!(stdout.is_empty(), "no stack on failure: {stdout:?}");
    assert!(
        stderr.contains("pushd: -n: No such file or directory"),
        "cd verdict: {stderr:?}"
    );
    assert!(
        !stderr.contains("usage:"),
        "cd failures print no usage line: {stderr:?}"
    );
}

/// A leading `--` disables rotation too: `-- +1` names a directory.
#[test]
fn pushd_double_dash_plus_n_is_a_directory_not_a_rotation() {
    let (code, stdout, stderr) = run("pushd -- +1\n");
    assert_eq!(code, Some(1), "rc for `pushd -- +1`: {stderr:?}");
    assert!(stdout.is_empty(), "no stack on failure: {stdout:?}");
    assert!(
        stderr.contains("pushd: +1: No such file or directory"),
        "cd verdict: {stderr:?}"
    );
}

/// A lone `--` still swaps; bare `pushd` is untouched.
#[test]
fn pushd_double_dash_alone_still_swaps() {
    let (code, stdout, stderr) = run("pushd / >/dev/null\npushd --\n");
    assert_eq!(code, Some(0), "rc for swap: {stderr:?}");
    assert!(stderr.is_empty(), "swap is quiet on stderr: {stderr:?}");
    assert!(
        stdout.trim_end().ends_with(" /"),
        "swap parks / in second place: {stdout:?}"
    );
}

/// Options with nothing left are the :288 silent success, never the swap.
#[test]
fn pushd_bare_n_forms_are_silent_noops() {
    for script in ["pushd -n\n", "pushd -n --\n"] {
        let (code, stdout, stderr) = run(script);
        assert_eq!(code, Some(0), "rc for {script:?}: {stderr:?}");
        assert!(stdout.is_empty(), "no-op prints nothing: {stdout:?}");
        assert!(stderr.is_empty(), "no-op is quiet: {stderr:?}");
    }
}

/// A lone `-` is `cd $OLDPWD`: echo the raw value, then the stack.
#[test]
fn pushd_dash_echoes_oldpwd_and_pushes() {
    let (code, stdout, stderr) = run("OLDPWD=/; pushd -\n");
    assert_eq!(code, Some(0), "rc for `pushd -`: {stderr:?}");
    assert!(stderr.is_empty(), "success is quiet: {stderr:?}");
    let mut lines = stdout.lines();
    assert_eq!(lines.next(), Some("/"), "raw OLDPWD echo: {stdout:?}");
    assert!(
        lines.next().is_some_and(|stack| stack.starts_with("/ ")),
        "stack after the echo: {stdout:?}"
    );
}

/// Unset OLDPWD refuses; set-but-empty is a no-op success with an echo.
#[test]
fn pushd_dash_oldpwd_edges() {
    let (code, _, stderr) = run("unset OLDPWD; pushd -\n");
    assert_eq!(code, Some(1), "rc for unset OLDPWD: {stderr:?}");
    assert!(
        stderr.contains("pushd: OLDPWD not set"),
        "cd verdict: {stderr:?}"
    );
    let (code, stdout, stderr) = run("OLDPWD=; pushd -\n");
    assert_eq!(code, Some(0), "rc for empty OLDPWD: {stderr:?}");
    assert!(stderr.is_empty(), "success is quiet: {stderr:?}");
    assert!(
        stdout.lines().next().is_some_and(str::is_empty),
        "empty OLDPWD still echoes a line: {stdout:?}"
    );
}

/// Multi-word Dir operands are cd's argc failure: message only, rc 1.
#[test]
fn pushd_extra_dir_words_are_too_many_arguments() {
    for script in [
        "pushd / extra\n",
        "pushd -- -n extra\n",
        "OLDPWD=/; pushd - -n\n",
        "pushd \"\" extra\n",
    ] {
        let (code, stdout, stderr) = run(script);
        assert_eq!(code, Some(1), "rc for {script:?}: {stderr:?}");
        assert!(stdout.is_empty(), "no stack on failure: {stdout:?}");
        assert!(
            stderr.contains("pushd: too many arguments"),
            "argc verdict: {stderr:?}"
        );
        assert!(
            !stderr.contains("usage:"),
            "argc failures print no usage line: {stderr:?}"
        );
    }
}

/// NOCD pushes word one verbatim and ignores the rest — even past `--`,
/// where `+1`/`-n` stop being options and rotations.
#[test]
fn pushd_n_pushes_first_word_verbatim() {
    let (code, stdout, stderr) = run("pushd -n a b\n");
    assert_eq!(code, Some(0), "rc: {stderr:?}");
    assert!(stderr.is_empty(), "success is quiet: {stderr:?}");
    assert!(
        stdout.trim_end().ends_with(" a"),
        "raw `a` on top: {stdout:?}"
    );
    let (code, stdout, stderr) = run("pushd -n -- +1\n");
    assert_eq!(code, Some(0), "rc: {stderr:?}");
    assert!(stderr.is_empty(), "success is quiet: {stderr:?}");
    assert!(
        stdout.trim_end().ends_with(" +1"),
        "literal `+1`, no rotation: {stdout:?}"
    );
    let (code, stdout, stderr) = run("pushd -n -- -n\n");
    assert_eq!(code, Some(0), "rc: {stderr:?}");
    assert!(stderr.is_empty(), "success is quiet: {stderr:?}");
    assert!(
        stdout.trim_end().ends_with(" -n"),
        "literal `-n`: {stdout:?}"
    );
    // With -n the lone `-` is a raw word too, never an OLDPWD lookup.
    let (code, stdout, stderr) = run("unset OLDPWD; pushd -n -\n");
    assert_eq!(code, Some(0), "rc: {stderr:?}");
    assert!(stderr.is_empty(), "no OLDPWD probe: {stderr:?}");
    assert!(stdout.trim_end().ends_with(" -"), "literal `-`: {stdout:?}");
}

/// `--` past the walk is literal as well: `pushd -- --` names a directory.
#[test]
fn pushd_double_dash_double_dash_is_a_directory() {
    let (code, stdout, stderr) = run("pushd -- --\n");
    assert_eq!(code, Some(1), "rc for `pushd -- --`: {stderr:?}");
    assert!(stdout.is_empty(), "no stack on failure: {stdout:?}");
    assert!(
        stderr.contains("pushd: --: No such file or directory"),
        "cd verdict: {stderr:?}"
    );
}
