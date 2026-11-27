//! niubash#282: console raw input support.
//!
//! The console-mode legs (single-key `read -n1` without Enter, `read -s`
//! echo suppression, `stty raw/-echo/sane/size`) need a real conhost
//! console and are covered by scripts/smoke-read-tty-pty.py under ConPTY;
//! the script SKIPs the stty legs on winpty-style bridges, where stdin is
//! a plain character device without the console-mode API (真机待验).
//! These are the deterministic pipe-transport legs: the engine builtin
//! answers stty itself (GNU bash runs an external stty; on Windows none
//! exists, so TUI scripts' `stty raw -echo` used to die with
//! command-not-found) and reports the coreutils non-terminal diagnostic.

use std::process::{Command, Stdio};

fn run_stdin(args: &[&str], stdin: &[u8]) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    use std::io::Write;
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

/// `stty` is claimed by the engine: a missing external can no longer break
/// TUI scripts' mode setup, and introspection keeps reporting external
/// (the name stays out of BUILTIN_NAMES), but the command resolves.
#[test]
fn stty_is_answered_by_engine() {
    let (stdout, stderr, rc) = run_stdin(&["-c", "command -v stty; echo \"rc=$?\""], b"");
    assert_ne!(stderr, "rubash: stty: command not found\n");
    assert!(rc.is_some());
    let _ = stdout;
}

/// Non-terminal stdin: the coreutils diagnostic and status 1 (GNU probe:
/// `stty raw < /dev/null` → `stty: 'standard input': Inappropriate ioctl
/// for device`, rc=1).
#[test]
fn stty_non_terminal_stdin_reports_gnu_diagnostic() {
    let (stdout, stderr, _rc) = run_stdin(&["-c", "stty raw; echo \"rc=$?\""], b"");
    assert_eq!(
        stderr, "stty: 'standard input': Inappropriate ioctl for device\n",
        "stdout: {stdout:?}"
    );
    assert_eq!(stdout, "rc=1\n");
}

/// Same for `stty size` and `stty -a` shapes — every operand form fails
/// with the diagnostic when stdin has no console-mode API.
#[test]
fn stty_size_non_terminal_diagnostic() {
    let (_stdout, stderr, rc) = run_stdin(&["-c", "stty size > /dev/null; echo \"rc=$?\""], b"");
    assert_eq!(
        stderr,
        "stty: 'standard input': Inappropriate ioctl for device\n"
    );
    assert_eq!(_stdout, "rc=1\n");
    let _ = rc;
}

/// Same diagnostic for the geometry query operands (`cols`/`rows`) and
/// `speed` — the non-terminal gate precedes operand handling, so every
/// query face fails identically on a pipe (niubash#535).
#[test]
fn stty_query_operands_non_terminal_diagnostic() {
    for operand in ["cols", "rows", "speed"] {
        let (stdout, stderr, _rc) =
            run_stdin(&["-c", &format!("stty {operand}; echo \"rc=$?\"")], b"");
        assert_eq!(
            stderr, "stty: 'standard input': Inappropriate ioctl for device\n",
            "operand {operand}, stdout: {stdout:?}"
        );
        assert_eq!(stdout, "rc=1\n", "operand {operand}");
    }
}

/// A shell function named stty keeps winning over the engine tool, like
/// every GNU external command (rubash#154 rule).
#[test]
fn shell_function_shadows_stty() {
    let (stdout, stderr, rc) =
        run_stdin(&["-c", "stty() { echo FUN; }; stty; echo \"rc=$?\""], b"");
    assert_eq!(stdout, "FUN\nrc=0\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}

// ---------------------------------------------------------------------
// Option parsing + dump faces (GNU coreutils stty 9.11 baselines, probed
// on WSL LC_ALL=C: stty.c:1287 main() extracts -a/-g from any position,
// stty.c:1379-1388 refuse the two option-style combinations, and
// stty.c:1392 apply_settings(checking) validates operands BEFORE the
// terminal is touched, so unknown operands are reported even on a pipe
// while well-formed ones fall through to the ioctl diagnostic).
// ---------------------------------------------------------------------

/// Pin (a): `stty -a` on a non-terminal reports the canned ioctl
/// diagnostic, NOT `invalid argument '-a'` (GNU T8/T14 shape: `--all`
/// and `--save` behave identically).
#[test]
fn stty_all_and_save_non_terminal_diagnostic() {
    for operand in ["-a", "--all", "-g", "--save"] {
        let (stdout, stderr, _rc) =
            run_stdin(&["-c", &format!("stty {operand}; echo \"rc=$?\"")], b"");
        assert_eq!(
            stderr, "stty: 'standard input': Inappropriate ioctl for device\n",
            "operand {operand}, stdout: {stdout:?}"
        );
        assert_eq!(stdout, "rc=1\n", "operand {operand}");
    }
}

/// Pin (b): the window-size SETTING form on a non-terminal reports the
/// canned ioctl diagnostic, NOT `invalid argument '40'` (GNU T1/T11).
#[test]
fn stty_window_size_setting_non_terminal_diagnostic() {
    for invocation in [
        "stty rows 40",
        "stty cols 120",
        "stty columns 80",
        "stty sane rows 40 cols 120",
    ] {
        let (stdout, stderr, _rc) =
            run_stdin(&["-c", &format!("{invocation}; echo \"rc=$?\"")], b"");
        assert_eq!(
            stderr, "stty: 'standard input': Inappropriate ioctl for device\n",
            "invocation {invocation}, stdout: {stdout:?}"
        );
        assert_eq!(stdout, "rc=1\n", "invocation {invocation}");
    }
}

/// The checking pass precedes the terminal gate (stty.c:1392): an
/// unknown operand is reported even on a pipe, with GNU's exact wording
/// plus the usage hint; a bad window-size VALUE is not — integer_arg
/// runs at apply time (stty.c:1220), so `rows abc` still reaches the
/// ioctl diagnostic (GNU P1).
#[test]
fn stty_checking_pass_precedes_terminal_gate() {
    let (stdout, stderr, _rc) = run_stdin(&["-c", "stty bogus; echo \"rc=$?\""], b"");
    assert_eq!(
        stderr, "stty: invalid argument 'bogus'\nTry 'stty --help' for more information.\n",
        "stdout: {stdout:?}"
    );
    assert_eq!(stdout, "rc=1\n");
    let (stdout, stderr, _rc) = run_stdin(&["-c", "stty sane rows abc; echo \"rc=$?\""], b"");
    assert_eq!(
        stderr, "stty: 'standard input': Inappropriate ioctl for device\n",
        "stdout: {stdout:?}"
    );
    assert_eq!(stdout, "rc=1\n");
}

/// Options and settings mix in any order (GNU main() getopt sweep): the
/// output styles refuse setting operands with the GNU wording, and the
/// two styles are mutually exclusive (stty.c:1379-1388, GNU T3/T4/T9).
#[test]
fn stty_output_style_refusals() {
    for invocation in [
        "stty -a sane",
        "stty -a bogus",
        "stty rows 40 -a",
        "stty -g size",
    ] {
        let (stdout, stderr, _rc) =
            run_stdin(&["-c", &format!("{invocation}; echo \"rc=$?\"")], b"");
        assert_eq!(
            stderr, "stty: when specifying an output style, modes may not be set\n",
            "invocation {invocation}, stdout: {stdout:?}"
        );
        assert_eq!(stdout, "rc=1\n", "invocation {invocation}");
    }
    for invocation in ["stty -a -g", "stty -g --all", "stty --save --all"] {
        let (stdout, stderr, _rc) =
            run_stdin(&["-c", &format!("{invocation}; echo \"rc=$?\"")], b"");
        assert_eq!(
            stderr,
            "stty: the options for verbose and stty-readable output styles are\nmutually exclusive\n",
            "invocation {invocation}, stdout: {stdout:?}"
        );
        assert_eq!(stdout, "rc=1\n", "invocation {invocation}");
    }
}

/// The checking pass admits the operand grammar GNU admits: bare baud
/// numbers from the string_to_baud table (GNU T5 valid, T6 rejected) and
/// the reversible mode bits like `-icanon` (GNU T13) fall through to the
/// ioctl diagnostic, while off-table tokens are rejected.
#[test]
fn stty_checking_pass_admits_gnu_grammar() {
    for invocation in [
        "stty 38400",
        "stty 0",
        "stty -icanon",
        "stty isig",
        "stty -- rows 40",
    ] {
        let (stdout, stderr, _rc) =
            run_stdin(&["-c", &format!("{invocation}; echo \"rc=$?\"")], b"");
        assert_eq!(
            stderr, "stty: 'standard input': Inappropriate ioctl for device\n",
            "invocation {invocation}, stdout: {stdout:?}"
        );
        assert_eq!(stdout, "rc=1\n", "invocation {invocation}");
    }
    for (invocation, token) in [
        ("stty 999999", "999999"),
        ("stty -sane", "-sane"),
        ("stty -cooked", "-cooked"),
    ] {
        let (stdout, stderr, _rc) =
            run_stdin(&["-c", &format!("{invocation}; echo \"rc=$?\"")], b"");
        assert_eq!(
            stderr,
            format!("stty: invalid argument '{token}'\nTry 'stty --help' for more information.\n"),
            "invocation {invocation}, stdout: {stdout:?}"
        );
        assert_eq!(stdout, "rc=1\n", "invocation {invocation}");
    }
}

/// Documented divergence (not pinned as GNU behavior): GNU admits the
/// value-taking control/`min`/`time`/`line` operands (stty.c control_info
/// — `stty min 1` passes its checking pass) and the WinuxCmd applet
/// accepts-and-ignores them; rubash rejects them as invalid arguments
/// until a c_cc model exists. This pin guards the deliberate rejection
/// so a future admit-and-ignore face changes it consciously.
#[test]
fn stty_value_operands_rejected_documented_divergence() {
    for invocation in ["stty min 1", "stty intr ^A", "stty line 0"] {
        let (stdout, stderr, _rc) =
            run_stdin(&["-c", &format!("{invocation}; echo \"rc=$?\"")], b"");
        assert!(
            stderr.starts_with("stty: invalid argument '"),
            "invocation {invocation}, stderr: {stderr:?}"
        );
        assert_eq!(stdout, "rc=1\n", "invocation {invocation}");
    }
}
