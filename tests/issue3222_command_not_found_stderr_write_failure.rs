//! rubash#3222: a failed write of an exec-error diagnostic must not
//! change the command's exit status.
//!
//! GNU semantics: execute_cmd.c shell_execve assigns
//! `last_command_exit_value = (i == ENOENT) ? EX_NOTFOUND : EX_NOEXEC`
//! (execute_cmd.c:6145) before any diagnostic is printed, and
//! error.c:207-218 `internal_error` ignores the return values of its
//! `vfprintf (stderr, ...)` calls — a dead stderr (e.g. redirected to
//! /dev/full) therefore leaves 127/126 untouched.
//!
//! rubash staged these diagnostics in a buffer and let
//! `finish_external_error` propagate the buffered-write error with
//! `?` before assigning `exit_code`, so a failed diagnostic write
//! flipped the command's status to 1. Every consumer of that finish
//! path is pinned here: command not found (127), slash-name execve
//! errors (127/126), and the exec-format refusals (126).

use std::process::{Command, Stdio};

fn run_rubash(script: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .stdin(Stdio::null())
        .output()
        .expect("spawn rubash")
}

fn status_of(output: &std::process::Output) -> i32 {
    output.status.code().expect("rubash exited with a code")
}

/// Control (all platforms): with a healthy stderr the diagnostic is
/// printed and the status is 127 — the pin's write-failure cells are
/// discriminating against this baseline, not against the diagnostic
/// path itself.
#[test]
fn cnfd_healthy_stderr_prints_diagnostic_and_keeps_127() {
    let output = run_rubash("nosuchcmd_3222_xyz");
    assert_eq!(status_of(&output), 127);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("command not found"),
        "expected command-not-found diagnostic, got: {stderr}"
    );
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::io::Write;

    fn stdout_of(script: &str) -> String {
        let output = run_rubash(script);
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// The shell process's own stderr is /dev/full: the diagnostic
    /// write fails outright, and the process exit status (the last
    /// command's status) must stay 127.
    #[test]
    fn cnfd_outer_stderr_dev_full_keeps_127() {
        let full = std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/full")
            .expect("/dev/full available on unix");
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .args(["-c", "nosuchcmd_3222_xyz"])
            .stdin(Stdio::null())
            .stderr(Stdio::from(full))
            .output()
            .expect("spawn rubash");
        assert_eq!(status_of(&output), 127);
    }

    /// In-shell redirect form: `$?` after the failed-write diagnostic
    /// is 127 and the script keeps running.
    #[test]
    fn cnfd_inner_redirect_dev_full_keeps_127_and_continues() {
        let stdout = stdout_of("nosuchcmd_3222_xyz 2>/dev/full; echo rc=$?; echo alive");
        assert!(stdout.contains("rc=127"), "stdout: {stdout}");
        assert!(stdout.contains("alive"), "stdout: {stdout}");
    }

    /// Slash-name ENOENT goes through the same finish path with the
    /// execve errno text; a failed diagnostic write keeps 127.
    #[test]
    fn slash_enoent_inner_redirect_dev_full_keeps_127() {
        let stdout = stdout_of("/nonexistent/zzz_3222 2>/dev/full; echo rc=$?");
        assert!(stdout.contains("rc=127"), "stdout: {stdout}");
    }

    /// A directory as the command word is EX_NOEXEC (126, EISDIR
    /// diagnostic) through the same finish path.
    #[test]
    fn directory_command_inner_redirect_dev_full_keeps_126() {
        let dir = std::env::temp_dir();
        let script = format!("{} 2>/dev/full; echo rc=$?", dir.display());
        let stdout = stdout_of(&script);
        assert!(stdout.contains("rc=126"), "stdout: {stdout}");
    }

    /// Control for the directory cell: healthy stderr keeps 126 too.
    #[test]
    fn directory_command_healthy_stderr_keeps_126() {
        let dir = std::env::temp_dir();
        let script = format!("{}; echo rc=$?", dir.display());
        let stdout = stdout_of(&script);
        assert!(stdout.contains("rc=126"), "stdout: {stdout}");
    }

    /// An executable file whose #! interpreter cannot be resolved is
    /// refused with "bad interpreter" (EX_NOEXEC 126) through the
    /// exec-format refusal arm of the same finish path; a failed
    /// diagnostic write must keep 126.
    #[test]
    fn bad_interpreter_inner_redirect_dev_full_keeps_126() {
        let dir = std::env::temp_dir().join("issue3222_badinterp_fixture");
        std::fs::create_dir_all(&dir).expect("create fixture dir");
        let script_path = dir.join("badinterp.sh");
        let mut file = std::fs::File::create(&script_path).expect("write fixture");
        file.write_all(b"#!/nonexistent/interp_3222\necho hi\n")
            .expect("write fixture body");
        drop(file);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod fixture");
        }
        // Healthy-stderr control first: the refusal itself is 126.
        let control = stdout_of(&format!("{}; echo rc=$?", script_path.display()));
        assert!(control.contains("rc=126"), "control stdout: {control}");
        let stdout = stdout_of(&format!(
            "{} 2>/dev/full; echo rc=$?",
            script_path.display()
        ));
        assert!(stdout.contains("rc=126"), "stdout: {stdout}");
    }
}
