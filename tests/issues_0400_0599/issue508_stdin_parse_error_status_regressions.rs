//! Issue rubash#508 regression: a syntax error in a stdin-driven script must
//! exit 2 like the identical file-driven run. The read-time parse-error fast
//! paths report the right diagnostic but returned a status the stdin driver
//! dropped — the shell then exited with the stale pre-error status (0).
//!
//! GNU spec (WSL GNU bash 5.3.0 probes, 2026-10-12; every fragment piped via
//! `bash -s`): all exit rc 2, error.c EX_SYNTAXERR — the reading channel
//! never changes the exit status (issue fragments F120/F123/F127/F128/F139,
//! plus F181's rc half from #516 whose message shape is a separate issue).
//!
//! ```text
//! $ printf 'echo "${x:-\n' | bash -s; echo $?
//! bash: line 1: unexpected EOF while looking for matching `}'
//! 2
//! ```
//!
//! Rust semantic owner: main.rs run_stdin_script — each gathered group's
//! status is committed to the executor before the driver looks at the
//! parse-error flag (GNU evalstring.c parse_and_execute leaves the group
//! status in last_command_exit_value for the reader to exit with).

#![cfg(windows)]

use std::io::Write;
use std::process::{Command, Stdio};

fn run_stdin(script: &str) -> (Option<i32>, String) {
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
    )
}

/// The five issue fragments, same root cause: unclosed quote/`${`/`$(` at
/// EOF — diagnostic already matched GNU, exit status now 2.
#[test]
fn issue_fragments_exit_2_via_stdin() {
    let cases: [&str; 5] = [
        "echo \"${x:-\n",                  // F120
        "x=axb; echo \"${x#\n",            // F123
        "$(m '\n",                         // F127
        "echo a\necho b\n'unterminated\n", // F128
        "echo \"$\n",                      // F139
    ];
    for script in cases {
        let (code, _stdout) = run_stdin(script);
        assert_eq!(code, Some(2), "stdin syntax error rc: {script:?}");
    }
}

/// #516 F181's rc half: the same exit-status pipeline — rc 2 via stdin (the
/// diagnostic shape divergence is #516's own parser issue, not asserted).
#[test]
fn f181_command_paren_rc_half_exits_2() {
    let (code, _stdout) = run_stdin("x='abc{def'; echo \\${x%%[<{(]*}\n");
    assert_eq!(code, Some(2));
}

/// A syntax error after successful groups still exits 2 (GNU rc 2), and the
/// shell stops reading there (no trailing group runs).
#[test]
fn mid_script_syntax_error_stops_reading_with_rc_2() {
    let (code, stdout) = run_stdin("echo a\necho \"${x:-\necho b\n");
    assert_eq!(code, Some(2));
    assert_eq!(stdout, "a\n");
}

/// The commit must not disturb ordinary group-status flow: a real exit, a
/// DISCARD-class arithmetic failure (rc 1 but reader continues), a failed
/// command followed by exit 0, and the plain-success run.
#[test]
fn ordinary_stdin_status_flow_unchanged() {
    let (code, _) = run_stdin("exit 5\n");
    assert_eq!(code, Some(5));

    // DISCARD: the group's status 1 survives as the next group's starting
    // $? but the reader continues; the final group's 0 is the exit status.
    let (code, stdout) = run_stdin("echo $((1/0))\necho after\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after\n");

    let (code, _) = run_stdin("false\nexit 0\n");
    assert_eq!(code, Some(0));

    let (code, stdout) = run_stdin("echo hi\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "hi\n");

    // A failed final group (no errexit) still exits with its status.
    let (code, _) = run_stdin("echo hi\nfalse\n");
    assert_eq!(code, Some(1));
}

/// File-driven runs were already correct and stay so (the drivers share the
/// run_source* status conversion; this locks the contract both must meet).
#[test]
fn file_driven_syntax_error_still_exits_2() {
    let dir = std::env::temp_dir().join(format!("rubash-issue508-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue508 scratch dir");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, "echo \"${x:-\n").expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash script");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(output.status.code(), Some(2));
}
