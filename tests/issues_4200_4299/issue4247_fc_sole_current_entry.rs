//! rubash#4247 — when the in-flight `fc` line is the sole history entry,
//! explicit listing operands (`fc -l 0`, `fc -l 1 2`, `fc -l -1`, even an
//! unmatched string) must list that entry with status 0 instead of
//! reporting `history specification out of range`.
//!
//! GNU anchors (bash-5.3 patch 20, vendored `third_party/bash`):
//! - `builtins/fc.def:334-350` fc_builtin computes
//!   `last_hist = i - rh - hist_last_line_added`, then clamps its own
//!   negative value to zero "per POSIX".
//! - `builtins/fc.def:593-600` fc_gethnum recomputes the same raw value
//!   and returns the ordinary integer `-1` when it is negative. That is
//!   not HIST_INVALID (`INT_MIN`, fc.def:91), so it survives the sentinel
//!   checks and is clamped by fc_builtin at `fc.def:395-399`.
//! - `builtins/fc.def:679-684` fc_gethist accepts only non-negative
//!   indices, so the execute form (`fc -s`) reports "no command found"
//!   in the same shape instead of recursively re-executing itself.
//!
//! Expectations were probed byte-for-byte against WSL GNU Bash 5.3.20
//! using LF script files with `HISTFILE` empty/isolated.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash4247-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

struct Probe {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_probe(script: &str, dir: &Path) -> Probe {
    std::fs::write(dir.join("probe.sh"), script).expect("write probe.sh");
    // `set -o history` loads HISTFILE on its first enable in GNU
    // (builtins/set.def:461-464). Point it at an empty per-test file so
    // the host user's history cannot become invisible prior entries.
    std::fs::write(dir.join("histfile"), b"").expect("write empty histfile");
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .arg("probe.sh")
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("HISTFILE", dir.join("histfile"));
    for key in [
        "BASH_ENV",
        "ENV",
        "WINUXSH_ROOT",
        "HISTCONTROL",
        "HISTIGNORE",
        "HISTSIZE",
        "HISTFILESIZE",
        "PROMPT_COMMAND",
    ] {
        command.env_remove(key);
    }
    for (key, _) in std::env::vars() {
        if key.starts_with("BASH_FUNC_") {
            command.env_remove(key);
        }
    }
    let mut child = command.spawn().expect("spawn rubash");
    let outcome = crate::common::wait_bounded(&mut child, Duration::from_secs(5));
    assert!(
        !outcome.timed_out,
        "rubash timed out (pid {}); stdout={:?} stderr={:?}",
        outcome.pid,
        String::from_utf8_lossy(&outcome.stdout),
        String::from_utf8_lossy(&outcome.stderr)
    );
    Probe {
        stdout: String::from_utf8_lossy(&outcome.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&outcome.stderr).into_owned(),
        code: outcome.code,
    }
}

fn sole_history_script(invocation: &str) -> String {
    format!("set -o history\n{invocation}\nprintf 'rc=<%s>\\n' \"$?\"\n")
}

#[test]
fn sole_current_entry_listing_specs_are_clamped_to_zero() {
    let dir = tempdir("list");
    for invocation in [
        "fc -l",
        "fc -l 0",
        "fc -l 1 2",
        "fc -l -1",
        "fc -l 99",
        "fc -l -99",
        "fc -l nosuchcmd",
    ] {
        let probe = run_probe(&sole_history_script(invocation), &dir);
        assert_eq!(probe.code, Some(0), "stderr: {}", probe.stderr);
        assert_eq!(probe.stderr, "", "invocation: {invocation}");
        assert_eq!(
            probe.stdout,
            format!("1\t {invocation}\nrc=<0>\n"),
            "invocation: {invocation}"
        );
    }
}

#[test]
fn sole_current_entry_execute_form_is_no_command_found() {
    let dir = tempdir("execute");
    for invocation in ["fc -s", "fc -s 0", "fc -s 1", "fc -s nosuchcmd"] {
        let probe = run_probe(&sole_history_script(invocation), &dir);
        assert_eq!(probe.code, Some(0), "stderr: {}", probe.stderr);
        assert_eq!(probe.stdout, "rc=<1>\n", "invocation: {invocation}");
        assert_eq!(
            probe.stderr, "probe.sh: line 2: fc: no command found\n",
            "invocation: {invocation}"
        );
    }
}

#[test]
fn prior_entry_control_keeps_zero_and_notfound_semantics() {
    let dir = tempdir("prior");
    let probe = run_probe(
        "set -o history\n:\nfc -l 0\nprintf 'rc=<%s>\\n' \"$?\"\n",
        &dir,
    );
    assert_eq!(probe.code, Some(0), "stderr: {}", probe.stderr);
    assert_eq!(probe.stderr, "");
    assert_eq!(probe.stdout, "1\t :\nrc=<0>\n");

    let probe = run_probe(
        "set -o history\n:\nfc -l nosuchcmd\nprintf 'rc=<%s>\\n' \"$?\"\n",
        &dir,
    );
    assert_eq!(probe.code, Some(0), "stderr: {}", probe.stderr);
    assert_eq!(probe.stdout, "rc=<1>\n");
    assert_eq!(probe.stderr, "probe.sh: line 3: fc: no command found\n");
}

#[test]
fn history_off_empty_and_pushed_shapes_match_gnu() {
    let dir = tempdir("off");
    let probe = run_probe(
        "set +o history\nfc -l 0\nprintf 'rc=<%s>\\n' \"$?\"\n",
        &dir,
    );
    assert_eq!(probe.code, Some(0), "stderr: {}", probe.stderr);
    assert_eq!(
        (probe.stdout.as_str(), probe.stderr.as_str()),
        ("rc=<0>\n", "")
    );

    let probe = run_probe(
        "set +o history\nhistory -s 'echo pushed1'\nfc -l 0\nprintf 'rc=<%s>\\n' \"$?\"\n",
        &dir,
    );
    assert_eq!(probe.code, Some(0), "stderr: {}", probe.stderr);
    assert_eq!(probe.stderr, "");
    assert_eq!(probe.stdout, "1\t echo pushed1\nrc=<0>\n");

    let probe = run_probe(
        "set +o history\nhistory -s 'echo pushed1'\nhistory -s 'echo pushed2'\nfc -l 1 2\nprintf 'rc=<%s>\\n' \"$?\"\n",
        &dir,
    );
    assert_eq!(probe.code, Some(0), "stderr: {}", probe.stderr);
    assert_eq!(probe.stderr, "");
    assert_eq!(probe.stdout, "1\t echo pushed1\n2\t echo pushed2\nrc=<0>\n");
}

#[test]
fn command_substitution_sees_the_recorded_current_line() {
    let dir = tempdir("comsub");
    let probe = run_probe(
        "set -o history\nout=$(fc -l 0)\nprintf 'rc=<%s> len=<%s> out=<%s>\\n' \"$?\" \"${#out}\" \"$out\"\n",
        &dir,
    );
    assert_eq!(probe.code, Some(0), "stderr: {}", probe.stderr);
    assert_eq!(probe.stderr, "");
    assert_eq!(probe.stdout, "rc=<0> len=<17> out=<1\t out=$(fc -l 0)>\n");

    let probe = run_probe(
        "set +o history\nout=$(fc -l 0)\nprintf 'rc=<%s> len=<%s> out=<%s>\\n' \"$?\" \"${#out}\" \"$out\"\n",
        &dir,
    );
    assert_eq!(probe.code, Some(0), "stderr: {}", probe.stderr);
    assert_eq!(probe.stderr, "");
    assert_eq!(probe.stdout, "rc=<0> len=<0> out=<>\n");
}
