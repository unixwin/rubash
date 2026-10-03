//! Console REPL channel regressions (rubash#419).
//!
//! A bare `rubash.exe` attached to a real console used to run a placeholder
//! REPL — a banner ("Rubash - A Rust implementation of GNU Bash"), a
//! hardcoded `$ ` prompt, magic `exit`/`quit` words and a "Goodbye!" line —
//! and ignored PS1 entirely (the wt53 themesweep had to pipe stdin through
//! a relay to reach the real renderer). The console case now routes through
//! the same interactive reader the piped path uses
//! (`run_interactive_stdin`), per GNU:
//!
//! * shell.c:541-547 — tty stdin + tty stderr (or `-i`) make the shell
//!   interactive; parse.y:1710-1711 — an interactive shell reads every
//!   command through yy_readline_get -> bashline.c:460-461, where
//!   `rl_outstream = stderr`: the EXPANDED PS1 renders on stderr for the
//!   tty case exactly as for the piped one.
//! * parse.y:6148-6160 prompt_again — the primary read decodes PS1, every
//!   continuation read decodes PS2 (read_secondary_line parse.y:2327).
//! * eval.c:203-207 reader_loop — a parse error sets EOF_Reached only
//!   `if (interactive == 0)`: an interactive shell survives a syntax
//!   error and keeps reading (a console typo must not kill the shell).
//!
//! The tty-specific pieces (single echo via the cooked console, the
//! ICRNL-style `\r\n` normalization) need a real console and are covered
//! by `scripts/smoke-console-repl-pty.py` (pywinpty + pyte, screens under
//! `target/issue-suites/results/wt64-repl/`); the tests here pin the
//! driver-level behavior reachable with piped stdin.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_stdin(args: &[&str], stdin: &[u8], env: &[(&str, &str)]) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(args)
        .env_remove("PS1")
        .env_remove("PS2")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env {
        cmd.env(key, value);
    }
    let mut child = cmd.spawn().expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(stdin)
        .expect("write stdin");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The continuation read inside one command renders the EXPANDED PS2
/// (parse.y:2327 read_secondary_line), not PS1: GNU piped-`-i` shows
/// `> ` before the `then`/`fi` lines of an open `if`.
#[test]
fn interactive_ps2_continuation_prompt_renders() {
    let (stdout, stderr, _) = run_stdin(&["-i"], b"if true\nthen\necho hi\nfi\nexit\n", &[]);
    assert_eq!(stdout, "hi\n");
    assert!(
        stderr.contains("\n> then\n"),
        "PS2 continuation missing from prompt stream: {stderr:?}"
    );
    assert!(
        stderr.contains("\n> fi\n"),
        "PS2 continuation missing before fi: {stderr:?}"
    );
}

/// A custom PS2 from the environment drives the continuation prompt
/// (prompt_again decodes *prompt_string_pointer, parse.y:6158-6159).
#[test]
fn interactive_custom_ps2_is_honored() {
    let (_stdout, stderr, _) = run_stdin(
        &["-i"],
        b"if true\nthen\n:\nfi\nexit\n",
        &[("PS2", "cont:: ")],
    );
    assert!(
        stderr.contains("cont:: then"),
        "custom PS2 not rendered: {stderr:?}"
    );
}

/// eval.c:203-207: an interactive shell survives a syntax error — the
/// diagnostic is reported and the NEXT command still runs. The old
/// behavior (parse abort ending the session) made every console typo
/// kill the shell.
#[test]
fn interactive_syntax_error_does_not_end_session() {
    let (stdout, stderr, code) = run_stdin(&["-i"], b"echo )bad\necho AFTER\nexit 5\n", &[]);
    assert!(stdout.contains("AFTER"), "stdout: {stdout:?}");
    assert!(
        stderr.contains("syntax error near unexpected token `)'"),
        "stderr: {stderr:?}"
    );
    assert_eq!(code, Some(5));
}

/// `-i -s` with piped stdin is GNU's readline reader too (verified WSL
/// GNU 5.3.0: `printf 'echo $-\n' | bash -i -s` renders PS1 on stderr):
/// the prompt channel renders and `$-` keeps the `himBHs` shape.
#[test]
fn interactive_dash_s_stdin_renders_prompt_stream() {
    let (stdout, stderr, _) = run_stdin(&["-i", "-s"], b"echo $-\nexit\n", &[]);
    assert!(stdout.contains("himBHs"), "stdout: {stdout:?}");
    assert!(
        stderr.contains("bash-5.3#"),
        "PS1 stream missing on -i -s: {stderr:?}"
    );
}

/// The placeholder artifacts are gone from the interactive reader: no
/// banner, no "Type 'exit' to quit", no "Goodbye!" (GNU prints nothing
/// at interactive startup).
#[test]
fn interactive_reader_has_no_placeholder_banner() {
    let (_stdout, stderr, code) = run_stdin(&["-i"], b"exit 0\n", &[]);
    assert!(
        !stderr.contains("Rubash - A Rust implementation"),
        "banner leaked: {stderr:?}"
    );
    assert!(
        !stderr.contains("Type 'exit' to quit"),
        "placeholder hint leaked: {stderr:?}"
    );
    assert!(
        !stderr.contains("Goodbye!"),
        "placeholder goodbye leaked: {stderr:?}"
    );
    assert_eq!(code, Some(0));
}

/// The placeholder REPL always exited 0; the real reader propagates the
/// `exit` builtin status (builtins/exit.def exit_builtin).
#[test]
fn interactive_exit_status_propagates() {
    let (_stdout, _stderr, code) = run_stdin(&["-i"], b"exit 3\n", &[]);
    assert_eq!(code, Some(3));
}

/// The PS1 environment variable drives the rendered prompt (the #419
/// complaint: PS1 was ignored on the console entry). GNU imports PS1
/// from the environment (variables.c set_if_not).
#[test]
fn interactive_env_ps1_drives_prompt_stream() {
    let (_stdout, stderr, _) = run_stdin(&["-i"], b"exit\n", &[("PS1", "X> ")]);
    assert!(
        stderr.starts_with("X> "),
        "env PS1 not rendered first: {stderr:?}"
    );
}

/// A bare invocation with piped stdin stays the promptless stdin-script
/// reader (shell.c:780-786): no prompt bytes on stderr, commands run,
/// `exit` status propagates.
#[test]
fn bare_piped_stdin_still_reads_as_script() {
    let (stdout, stderr, code) = run_stdin(&[], b"echo A\nexit 9\n", &[]);
    assert_eq!(stdout, "A\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr:?}");
    assert_eq!(code, Some(9));
}
