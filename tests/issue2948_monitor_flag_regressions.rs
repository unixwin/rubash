//! rubash#2948 — the `m` letter in `$-` tracks job-control
//! establishment, not the interactive bit. GNU jobs.c:4731-4869
//! `initialize_job_control` (run at startup for interactive shells,
//! shell.c:1969) ends with jobs.c:4869
//! `change_flag ('m', job_control ? '-' : '+')`: `m` is present
//! exactly when the shell could take the terminal, and establishment
//! failure prints the two diagnostics (jobs.c:4841 "cannot set
//! terminal process group" and jobs.c:4856 "no job control in this
//! shell"). Measured against GNU bash 5.3.20:
//!
//! | form | GNU | rubash before |
//! |---|---|---|
//! | `-i -c` (no tty) | `hiBHc` + diagnostics | `himBHc` |
//! | `-i` stdin pipe (no tty) | `hiBHs` + diagnostics | `himBHs` |
//! | `-i script` (no tty) | `hiBH` + diagnostics | `himBH` |
//! | `-i script` under PTY | `himBH` | `himBH` |
//! | `-i -m -c` (no tty) | `hiBHc` + diagnostics | (explicit -m kept) |
//! | `+m -i -c` (no tty) | `hiBHc` + diagnostics | — |
//! | `-m -c` (no tty) | `hBc` + diagnostics | `hBc` (already right) |
//!
//! Harness shapes (recorded per PTY discipline):
//! - no-tty cells: all three stdio streams are piped, and on unix
//!   the child is setsid-detached in pre_exec, so it has NO
//!   controlling terminal at all — /dev/tty cannot open — on any
//!   host, desktop included.
//! - PTY cell: util-linux `script -qec` (the issue1016 harness):
//!   the shell runs as session leader on a fresh pty, foreground
//!   process group, controlling terminal = the pty slave. The cell
//!   pins establishment only; no signal-delivery assertions.

use std::io::Write;
use std::process::{Command, Stdio};

fn rubash_no_tty(args: &[&str], stdin_bytes: &[u8]) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn().expect("spawn rubash");
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(stdin_bytes);
    }
    let out = child.wait_with_output().expect("wait for rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

fn assert_no_job_control_diagnostics(stderr: &str) {
    assert!(
        stderr.contains("cannot set terminal process group"),
        "GNU prints the terminal-process-group diagnostic when job \
         control cannot be established, got stderr: {stderr:?}"
    );
    assert!(
        stderr.contains("no job control in this shell"),
        "GNU prints the no-job-control diagnostic when job control \
         cannot be established, got stderr: {stderr:?}"
    );
}

#[test]
fn interactive_command_no_terminal_drops_m() {
    let (stdout, stderr, _) = rubash_no_tty(&["-i", "-c", "echo FLAGS=$-"], b"");
    assert_eq!(stdout.trim(), "FLAGS=hiBHc", "stderr: {stderr:?}");
    assert_no_job_control_diagnostics(&stderr);
}

#[test]
fn interactive_stdin_pipe_no_terminal_drops_m() {
    let (stdout, stderr, _) = rubash_no_tty(&["-i"], b"echo FLAGS=$-\nexit\n");
    assert!(
        stdout.contains("FLAGS=hiBHs"),
        "GNU: hiBHs without a terminal, got stdout: {stdout:?} stderr: {stderr:?}"
    );
    assert_no_job_control_diagnostics(&stderr);
}

#[test]
fn interactive_script_no_terminal_drops_m() {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i2948-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let script = dir.join("probe.sh");
    std::fs::write(&script, "echo FLAGS=$-\n").expect("write script");
    let script_arg = script.to_string_lossy().into_owned();
    let (stdout, stderr, _) = rubash_no_tty(&["-i", &script_arg], b"");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        stdout.contains("FLAGS=hiBH"),
        "GNU: hiBH without a terminal, got stdout: {stdout:?} stderr: {stderr:?}"
    );
    assert!(
        !stdout.contains("FLAGS=himBH"),
        "m must not appear without a terminal, got stdout: {stdout:?}"
    );
    assert_no_job_control_diagnostics(&stderr);
}

#[test]
fn interactive_explicit_monitor_flags_no_terminal_still_drop_m() {
    // GNU: an explicit -m does not survive a terminal that cannot be
    // taken, and an explicit +m does not change the diagnostics —
    // initialize_job_control's result decides `m` wholesale.
    for args in [
        vec!["-i", "-m", "-c", "echo FLAGS=$-"],
        vec!["+m", "-i", "-c", "echo FLAGS=$-"],
    ] {
        let (stdout, stderr, _) = rubash_no_tty(&args, b"");
        assert_eq!(
            stdout.trim(),
            "FLAGS=hiBHc",
            "args {args:?}: GNU drops m without a terminal, stderr: {stderr:?}"
        );
        assert_no_job_control_diagnostics(&stderr);
    }
}

#[test]
fn noninteractive_explicit_monitor_no_terminal_unchanged() {
    // Control: the non-interactive -m arm was already GNU-shaped
    // (two diagnostics + hBc); the interactive fix must not move it.
    let (stdout, stderr, _) = rubash_no_tty(&["-m", "-c", "echo FLAGS=$-"], b"");
    assert_eq!(stdout.trim(), "FLAGS=hBc", "stderr: {stderr:?}");
    assert_no_job_control_diagnostics(&stderr);
}

#[cfg(unix)]
#[test]
fn interactive_script_under_pty_keeps_m() {
    if Command::new("script").arg("--version").output().is_err() {
        eprintln!("no script(1) on this host; skipping");
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "rubash-i2948-pty-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let script = dir.join("probe.sh");
    std::fs::write(&script, "echo FLAGS=$-\n").expect("write script");
    let cmdline = format!(
        "{} -i {}",
        env!("CARGO_BIN_EXE_rubash"),
        script.to_string_lossy()
    );
    let out = Command::new("script")
        .args(["-qec", &cmdline, "/dev/null"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn script(1)");
    let _ = std::fs::remove_dir_all(&dir);
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    assert!(
        text.contains("FLAGS=himBH"),
        "GNU: himBH with a terminal, got transcript: {text:?}"
    );
    assert!(
        !text.contains("no job control in this shell"),
        "no establishment diagnostics under a terminal, got transcript: {text:?}"
    );
}
