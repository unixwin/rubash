//! Subshell + `;` + stray `)` (`(A;B);)`) regressions (rubash#845).
//!
//! #845 claimed GNU accepts the `(A;B);)` tail in an interactive PS2
//! continuation while rubash reports a syntax error, interactive-only,
//! with `-c`/stdin script accepting the same shape. Measured against the
//! sole baseline (WSL GNU Bash 5.3.0, `/usr/local/bin/bash`), the premise
//! is false on every leg — the "#426 GNU 5.3 msys oracle" binary
//! (`winuxcmd/usr/bin/bash.exe`, version string `x86_64-pc-msys`) is a
//! 243 KB launcher whose embedded strings read `bash shim: cannot locate
//! niu.exe; set NIU_SHELL to its path`, i.e. it runs NIUBASH, so its
//! "accepts" verdict was niubash's own `-c`/stdin behavior echoed back.
//!
//! What GNU bash actually does (all verified live, plus the grammar):
//! `simple_list: simple_list1 ';'` (parse.y:1377-1394) only reduces when
//! the lookahead is a terminator (`'\n'`/yacc_EOF, simple_list_terminator
//! parse.y:1310-1314); a `)` cannot start `simple_list1` (parse.y:1389),
//! so `;` followed by `)` is a syntax error and the subshell never runs.
//!
//! * `bash -c '(echo one; echo two);)'` -> `-c: line 1: syntax error near
//!   unexpected token ')'`, rc 2, nothing runs. Multiline `-c` payload
//!   `(echo one;\necho two);)` -> same error attributed to `line 2`.
//! * stdin script one-liner -> `line 1: syntax error ...`, rc 2, and the
//!   shell exits before the next command (a script dies on the first
//!   parse error; verified: `echo a\n)\necho b` runs only `echo a`).
//! * stdin script `(echo one;\necho two);\n)` -> the `;`+NEWLINE completes
//!   the list, the subshell RUNS (one, two), then the stray `)` line is
//!   `line 3: syntax error ...`, rc 2, `AFTER-OK` never reached.
//! * interactive `-i` -> the diagnostic is reported, NOTHING executes, and
//!   the session survives (eval.c:203-207 reader_loop: EOF_Reached only
//!   `if (interactive == 0)`); with the split after `);` the subshell
//!   runs at the PS2 boundary and the stray `)` errors separately.
//!   Verified on a real PTY (`script -qec bash -i`) for GNU and a real
//!   ConPTY for rubash; the piped `-i` legs below pin the same reader
//!   (`run_interactive_stdin` serves tty and non-tty alike, parse.y:1710).
//!
//! Rubash already matches GNU on all of these — this file pins the
//! conformance so the family cannot silently drift in either direction.

use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static RUN_SEQ: AtomicUsize = AtomicUsize::new(0);

/// Hermetic runner: empty HOME (no rc, no inputrc), PS1/PS2 scrubbed, so
/// the default prompt and diagnostics are host-independent. The HOME dir
/// is unique per call — parallel test threads share a process id.
fn run_shell(args: &[&str], stdin: &[u8]) -> (String, String, Option<i32>) {
    let seq = RUN_SEQ.fetch_add(1, Ordering::Relaxed);
    let home_dir =
        std::env::temp_dir().join(format!("rubash-845-home-{}-{}", std::process::id(), seq));
    let _ = std::fs::remove_dir_all(&home_dir);
    std::fs::create_dir_all(&home_dir).expect("empty HOME");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.env("HOME", &home_dir)
        .env_remove("PS1")
        .env_remove("PS2")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(stdin)
        .expect("write stdin");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait rubash");
    let _ = std::fs::remove_dir_all(&home_dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// `-c` one-liner: `simple_list1 ';'` + lookahead `)` is a syntax error;
/// the subshell never runs (GNU rc 2, empty stdout).
#[test]
fn c_mode_one_line_stray_close_matches_gnu() {
    let (stdout, stderr, code) = run_shell(&["-c", "(echo one; echo two);)"], b"");
    assert_eq!(stdout, "", "the subshell must not run: {stdout:?}");
    assert_eq!(code, Some(2), "stderr: {stderr:?}");
    assert!(
        stderr.contains("bash: -c: line 1: syntax error near unexpected token `)'"),
        "stderr: {stderr:?}"
    );
    assert!(
        stderr.contains("-c: line 1: `(echo one; echo two);)'"),
        "offending-line echo missing: {stderr:?}"
    );
}

/// `-c` payload spanning the PS2 boundary shape (`(echo one;\necho
/// two);)`): GNU attributes the error to `line 2` and echoes the
/// offending physical line.
#[test]
fn c_mode_multiline_continuation_shape_matches_gnu() {
    let (stdout, stderr, code) = run_shell(&["-c", "(echo one;\necho two);)"], b"");
    assert_eq!(stdout, "", "the subshell must not run: {stdout:?}");
    assert_eq!(code, Some(2), "stderr: {stderr:?}");
    assert!(
        stderr.contains("bash: -c: line 2: syntax error near unexpected token `)'"),
        "stderr: {stderr:?}"
    );
    assert!(
        stderr.contains("-c: line 2: `echo two);)'"),
        "offending-line echo missing: {stderr:?}"
    );
}

/// stdin script one-liner: rc 2 and the shell dies before the NEXT command
/// (GNU: `echo a\n)\necho b` runs only `echo a`).
#[test]
fn stdin_script_stray_close_stops_before_next_command() {
    let (stdout, stderr, code) = run_shell(&[], b"(echo one; echo two);)\necho AFTER-OK\n");
    assert_eq!(
        stdout, "",
        "neither subshell nor later commands may run: {stdout:?}"
    );
    assert_eq!(code, Some(2), "stderr: {stderr:?}");
    assert!(
        stderr.contains("line 1: syntax error near unexpected token `)'"),
        "stderr: {stderr:?}"
    );
}

/// stdin script split at the `;`+NEWLINE boundary: the subshell RUNS
/// (`;`+newline completes the list, parse.y:1377-1394), the stray `)` line
/// is `line 3: syntax error ...`, and the script exits 2 without reaching
/// `AFTER-OK`.
#[test]
fn stdin_script_close_semicolon_runs_then_stray_errors() {
    let (stdout, stderr, code) = run_shell(&[], b"(echo one;\necho two);\n)\necho AFTER-OK\n");
    assert_eq!(stdout, "one\ntwo\n", "stdout: {stdout:?}");
    assert_eq!(code, Some(2), "stderr: {stderr:?}");
    assert!(
        stderr.contains("line 3: syntax error near unexpected token `)'"),
        "stderr: {stderr:?}"
    );
}

/// Interactive PS2 with the stray close on the continuation line: PS2
/// renders (read_secondary_line, parse.y:2327), the diagnostic is
/// reported, NOTHING executes, and the session survives (eval.c:203-207).
#[test]
fn interactive_ps2_stray_close_errors_and_session_survives() {
    let (stdout, stderr, code) =
        run_shell(&["-i"], b"(echo one;\necho two);)\necho AFTER-OK\nexit\n");
    assert_eq!(stdout, "AFTER-OK\n", "subshell output leaked: {stdout:?}");
    assert_eq!(code, Some(0), "session must survive: {stderr:?}");
    assert!(
        stderr.contains("> echo two);)"),
        "PS2 continuation missing from prompt stream: {stderr:?}"
    );
    assert!(
        stderr.contains("bash: syntax error near unexpected token `)'"),
        "stderr: {stderr:?}"
    );
    assert!(
        stderr.contains(" echo AFTER-OK"),
        "next primary prompt echo missing: {stderr:?}"
    );
}

/// Interactive PS2 split after `);`: the subshell runs at the PS2
/// boundary, then the stray `)` line errors and the session STILL
/// survives (the interactive reader_loop never turns a parse error into
/// EOF).
#[test]
fn interactive_ps2_close_semicolon_runs_then_stray_survives() {
    let (stdout, stderr, code) =
        run_shell(&["-i"], b"(echo one;\necho two);\n)\necho AFTER-OK\nexit\n");
    assert_eq!(stdout, "one\ntwo\nAFTER-OK\n", "stdout: {stdout:?}");
    assert_eq!(code, Some(0), "session must survive: {stderr:?}");
    assert!(
        stderr.contains("> echo two);"),
        "PS2 continuation missing from prompt stream: {stderr:?}"
    );
    assert!(
        stderr.contains("bash: syntax error near unexpected token `)'"),
        "stderr: {stderr:?}"
    );
    assert!(
        stderr.contains(" echo AFTER-OK"),
        "next primary prompt echo missing: {stderr:?}"
    );
}
