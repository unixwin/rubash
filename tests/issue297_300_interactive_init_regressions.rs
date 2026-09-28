//! Issue rubash#297 and rubash#300 regressions: interactive initialization.
//!
//! #297: `exit` inside a `--rcfile` startup file terminates the shell
//! (GNU builtins/exit.def exit_builtin -> exit_shell unwinds through the
//! run_startup_files evalstring; shell.c:638-654's setjmp_sigs catch runs
//! exit_shell at :642) — nothing after the rcfile runs: not the interactive
//! reader, not the remaining stdin, not a pending `-c` string or script.
//! The `exit`/`logout` echo (exit.def:59-62) does not fire for a
//! startup-file exit because GNU's `interactive` C global is 0 during the
//! startup files (verified with gdb on WSL GNU 5.3.0: exit_builtin inside a
//! --rcfile sees interactive=0, interactive_shell=1).
//!
//! #300: an interactive shell binds COLUMNS/LINES before prompt-time code
//! runs (rubash previously left them unset, so oh-my-bash themes' prompt
//! arithmetic `$(($COLUMNS/1))` failed with "arithmetic syntax error").
//! GNU binds from the controlling terminal during initialize_job_control
//! (jobs.c:4871 -> jobs.c:2647-2648 -> winsize.c:98-100, pre-startup-files)
//! and again at readline's first-prompt initialization
//! (bashline.c:524 -> readline.c:1313 -> terminal.c:293-374
//! _rl_get_screen_size), whose no-terminal fallback is 80x24
//! (terminal.c:366-371). Non-interactive shells never bind them.
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-28/29): `A`-only
//! stdout, empty stderr, rc 7 for `exit 7` rcfiles across `-i`, `-i -s`,
//! and `-i -c`; `bash-5.3# exit` on stderr for the reader shape.

use std::io::Write;
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn rcfile(name: &str, contents: &str) -> String {
    let path = std::env::temp_dir().join(format!("rubash-297-300-{}-{}", name, std::process::id()));
    std::fs::write(&path, contents).expect("write rcfile");
    path.to_string_lossy().into_owned()
}

fn run_interactive(rc: &str, args: &[&str], stdin: &[u8]) -> Run {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("--rcfile")
        .arg(rc)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(stdin)
        .expect("write stdin");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn run_simple(args: &[&str], stdin: &[u8]) -> Run {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(stdin)
        .expect("write stdin");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

// ---------------------------------------------------------------------------
// rubash#297: rcfile exit terminates the interactive shell
// ---------------------------------------------------------------------------

/// The reporter's shape: `exit 0` in the rcfile, one stdin command line.
/// GNU: stdout `A`, stderr empty, rc 0 — `echo C` is never read (WSL GNU
/// 5.3.0, probe t2).
#[test]
fn rcfile_exit_terminates_before_stdin_reader() {
    let rc = rcfile("t2", "echo A\nexit 0\necho B\n");
    let run = run_interactive(&rc, &["-i"], b"echo C\n");
    assert_eq!(run.stdout, "A\n");
    assert!(run.stderr.is_empty(), "stderr not empty: {:?}", run.stderr);
    assert_eq!(run.code, Some(0));
}

/// `exit 7` in the rcfile propagates the status; the rest of the rcfile
/// (`echo B`) and the stdin line never run. GNU: `A`, rc 7 (probe t2b).
#[test]
fn rcfile_exit_status_propagates_and_skips_rest() {
    let rc = rcfile("t2b", "echo A\nexit 7\necho B\n");
    let run = run_interactive(&rc, &["-i"], b"echo C\n");
    assert_eq!(run.stdout, "A\n");
    assert!(run.stderr.is_empty(), "stderr not empty: {:?}", run.stderr);
    assert_eq!(run.code, Some(7));
}

/// The `-i -c` form: the rcfile exit wins over the command string.
/// GNU: `A` only, rc 7 — `echo XC` never executes (probe ic).
#[test]
fn rcfile_exit_beats_pending_dash_c_string() {
    let rc = rcfile("t2c", "echo A\nexit 7\n");
    let run = run_interactive(&rc, &["-i", "-c", "echo XC"], b"");
    assert_eq!(run.stdout, "A\n");
    assert!(run.stderr.is_empty(), "stderr not empty: {:?}", run.stderr);
    assert_eq!(run.code, Some(7));
}

/// The `-i -s` form: the rcfile exit wins over the stdin reader.
/// GNU: `A`, rc 7 (probe is).
#[test]
fn rcfile_exit_beats_dash_s_stdin_reader() {
    let rc = rcfile("t2d", "echo A\nexit 7\n");
    let run = run_interactive(&rc, &["-i", "-s"], b"echo C\n");
    assert_eq!(run.stdout, "A\n");
    assert!(run.stderr.is_empty(), "stderr not empty: {:?}", run.stderr);
    assert_eq!(run.code, Some(7));
}

/// The exit.def:59-62 `exit` echo fires for a reader-phase exit but NOT for
/// a startup-file exit (GNU's `interactive` C global is 0 while the startup
/// files run). GNU: `bash -i -c 'exit 3'` -> stderr `exit\n`; rcfile exit
/// -> stderr empty.
#[test]
fn exit_echo_suppressed_in_rcfile_but_not_reader_phase() {
    let reader = run_simple(&["-i", "-c", "exit 3"], b"");
    assert_eq!(reader.stderr, "exit\n");
    assert_eq!(reader.code, Some(3));

    let rc = rcfile("t2e", "exit 0\n");
    let startup = run_interactive(&rc, &["-i"], b"");
    assert!(
        startup.stderr.is_empty(),
        "stderr not empty: {:?}",
        startup.stderr
    );
    assert_eq!(startup.code, Some(0));
}

// ---------------------------------------------------------------------------
// rubash#300: interactive COLUMNS/LINES binding
// ---------------------------------------------------------------------------

/// The reporter's shape (OMB iterate theme): `$((COLUMNS/1))` and
/// `$(($COLUMNS/1))` in a PROMPT_COMMAND must evaluate without the
/// "arithmetic syntax error" that an unset COLUMNS produced. Console or
/// 80x24-default, COLUMNS is positive by prompt time under GNU; rubash#300
/// previously left it unset (probe t3: GNU r=80 d=80 rc 0).
#[test]
fn interactive_columns_feeds_prompt_arithmetic() {
    let rc = rcfile(
        "t3",
        "PROMPT_COMMAND='echo r=$((COLUMNS/1)) d=$(($COLUMNS/1))'\n",
    );
    let run = run_interactive(&rc, &["-i"], b"");
    let r: i32 = run
        .stdout
        .lines()
        .find_map(|l| l.strip_prefix("r="))
        .and_then(|l| l.split(' ').next())
        .and_then(|v| v.parse().ok())
        .unwrap_or(-1);
    assert!(r > 0, "COLUMNS-derived arithmetic failed: {:?}", run.stdout);
    assert!(
        !run.stderr.contains("arithmetic syntax error"),
        "arithmetic error on stderr: {:?}",
        run.stderr
    );
    assert_eq!(run.code, Some(0));
}

/// A command line read by the interactive reader sees COLUMNS and LINES
/// bound (GNU binds LINES/COLUMNS by the first prompt read; the no-terminal
/// fallback is 80x24, terminal.c:366-371). Probe p3: GNU `IN:C=80/L=1`
/// from its console; without a console both fall back to 80/24.
#[test]
fn interactive_reader_sees_columns_and_lines_bound() {
    let rc = rcfile("t3b", "true\n");
    let run = run_interactive(&rc, &["-i"], b"echo IN:${COLUMNS-UNSET}/${LINES-UNSET}\n");
    let line = run.stdout.lines().last().unwrap_or_default().to_string();
    let values: Vec<&str> = line
        .strip_prefix("IN:")
        .map(|rest| rest.split('/').collect())
        .unwrap_or_default();
    assert_eq!(values.len(), 2, "unexpected output: {:?}", run.stdout);
    let columns: i32 = values[0].parse().unwrap_or(-1);
    let lines: i32 = values[1].parse().unwrap_or(-1);
    assert!(columns > 0, "COLUMNS unset or invalid: {line}");
    assert!(lines > 0, "LINES unset or invalid: {line}");
}

/// Non-interactive shells never bind COLUMNS/LINES (GNU jobs.c:4871 gates
/// get_tty_state on `interactive`; the readline init never runs).
#[test]
fn noninteractive_shells_do_not_bind_columns() {
    let run = run_simple(&["-c", "echo ${COLUMNS-UNSET}"], b"");
    // A console-carrying parent may have COLUMNS in its environment, which
    // the child inherits through getenv; only the no-COLUMNS parent can
    // assert the UNSET shape.
    if std::env::var_os("COLUMNS").is_none() {
        assert_eq!(run.stdout, "UNSET\n");
    }
}

/// The reader shape renders the expanded default prompt exactly like GNU's
/// non-tty readline path: `bash --rcfile /dev/null -i </dev/null` leaves
/// `bash-5.3# exit\n` on stderr (clean-env WSL GNU 5.3.0, probe o4).
#[test]
fn interactive_reader_echoes_expanded_default_prompt_at_eof() {
    let run = run_simple(&["--rcfile", "/dev/null", "-i"], b"");
    assert_eq!(run.stderr, "bash-5.3# exit\n");
    assert_eq!(run.code, Some(0));
}

/// A PS1 set by the rcfile is echoed EXPANDED with raw-byte markers (ESC
/// carriers) rendered as real bytes — GNU's prompt_again (parse.y:6158)
/// passes PS1 through decode_prompt_string before readline displays it.
#[test]
fn interactive_reader_echoes_expanded_ps1_with_esc_carriers_decoded() {
    let rc = rcfile("t3c", "_t=$'\\033[36m'\nPS1=\"${_t}(\"\nexit 0\n");
    let run = run_interactive(&rc, &["-i"], b"");
    // The rcfile exits, so the reader never runs and stderr stays clean;
    // the exit itself must not have parsed or echoed the PS1 text.
    assert!(run.stderr.is_empty(), "stderr not empty: {:?}", run.stderr);
    assert_eq!(run.code, Some(0));
}
