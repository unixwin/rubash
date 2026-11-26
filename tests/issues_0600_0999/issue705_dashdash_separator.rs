//! unixwin/rubash#705 R7.2: the `--` (and lone `-`) end-of-options separator.
//!
//! GNU shell.c:905-911 returns from `parse_shell_options` on a lone `-` or
//! `--`; the following word (if any) is the script file and, with none, the
//! shell reads standard input (shell.c:780-786 sets `read_from_stdin`).
//! `ShellInvocation::parse` used to reject `--` with no operand
//! (`--: option requires a script`) and never reached the stdin path, and a
//! lone `-` fell through to the invalid-option arm. The missing-script
//! diagnostic after `--` is the normal file_error path (rc 127), pinned here
//! for the engine route.
//!
//! Oracle: WSL GNU bash 5.3.0(1)-release. `printf 'echo X\n' | bash --`
//! prints `X` rc 0; `bash -- nosuch.sh` prints
//! `bash: nosuch.sh: No such file or directory` rc 127.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn sandbox(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-705-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create sandbox dir");
    dir
}

fn engine(dir: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .current_dir(dir)
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .env("HOME", dir);
    command
}

fn run(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let output = engine(dir).args(args).output().expect("spawn rubash");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn run_with_stdin(dir: &Path, args: &[&str], stdin: &str) -> (i32, String, String) {
    let mut child = engine(dir)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait rubash");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn invocation(args: &[&str]) -> rubash::invocation::ShellInvocation {
    rubash::invocation::ShellInvocation::parse(
        &args
            .iter()
            .map(|arg| (*arg).to_string())
            .collect::<Vec<_>>(),
    )
    .expect("parse invocation")
}

/// The parser contract the host consumes: `--` / `-` with no operand reads
/// stdin, with an operand it is the script and the rest are positionals.
#[test]
fn bare_separator_requests_stdin_and_operand_is_the_script() {
    for separator in ["-", "--"] {
        let parsed = invocation(&[separator]);
        assert!(parsed.script.is_none(), "{separator}: no script");
        assert!(parsed.read_stdin, "{separator}: must read stdin");
        assert!(parsed.positional_params.is_empty());

        let parsed = invocation(&[separator, "s.sh"]);
        assert_eq!(parsed.script.as_deref(), Some("s.sh"), "{separator}");
        assert!(!parsed.read_stdin);
        assert!(parsed.positional_params.is_empty());

        let parsed = invocation(&[separator, "s.sh", "a", "b"]);
        assert_eq!(parsed.script.as_deref(), Some("s.sh"), "{separator}");
        assert_eq!(parsed.positional_params, vec!["a", "b"]);
    }
}

/// Options before the separator still apply (GNU parses until it sees it).
#[test]
fn options_before_the_separator_still_apply() {
    let parsed = invocation(&["-e", "--", "s.sh", "x"]);
    assert!(parsed.shell_flags.contains(&("errexit".into(), true)));
    assert_eq!(parsed.script.as_deref(), Some("s.sh"));
    assert_eq!(parsed.positional_params, vec!["x"]);
}

/// Engine route: `rubash --` with piped stdin runs the input (GNU reads it).
#[test]
fn engine_separator_with_piped_stdin_runs_the_input() {
    let dir = sandbox("dashdash-stdin");
    for separator in ["-", "--"] {
        let (rc, stdout, stderr) = run_with_stdin(&dir, &[separator], "echo X\n");
        assert_eq!(
            (rc, stdout.as_str(), stderr.as_str()),
            (0, "X\n", ""),
            "{separator}"
        );
    }
}

/// Engine route: a missing script after `--` uses the normal 127 diagnostic,
/// not a raw OS error.
#[test]
fn engine_missing_script_after_separator_exits_127() {
    let dir = sandbox("dashdash-missing");
    for separator in ["-", "--"] {
        let (rc, stdout, stderr) = run(&dir, &[separator, "nosuch.sh"]);
        assert_eq!(rc, 127, "{separator}");

        assert_eq!(stdout, "");
        assert!(
            stderr.ends_with("nosuch.sh: No such file or directory\n"),
            "{separator}: {stderr:?}"
        );
    }
}

/// Engine route: a script named after the separator runs with its operands.
#[test]
fn engine_script_after_separator_runs_with_positionals() {
    let dir = sandbox("dashdash-script");
    fs::write(dir.join("dd.sh"), "printf '%s:%s\\n' \"$0\" \"$1\"\n").expect("write script");
    for separator in ["-", "--"] {
        let (rc, stdout, stderr) = run(&dir, &[separator, "dd.sh", "one"]);
        assert_eq!(
            (rc, stdout.as_str(), stderr.as_str()),
            (0, "dd.sh:one\n", ""),
            "{separator}"
        );
    }
}
