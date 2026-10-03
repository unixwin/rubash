//! unixwin/niubash#162 regression (rubash engine side): the TMPDIR a child
//! process receives must be usable by FOREIGN Windows executables.
//!
//! bunfs evidence: a Bun-compiled standalone executable (the opencode TUI)
//! prefers TMPDIR over TEMP/TMP when it extracts its embedded native DLLs.
//! A slash-drive `/c/...` TMPDIR is not a Windows path — Bun resolves it
//! drive-relative, the `B:` virtual-drive extraction fails, and dlopen falls
//! back to the embedded virtual path `B:/~BUN/root/opentui-*.dll` ->
//! ERROR_MOD_NOT_FOUND (126). Native `C:/...` (and an absent TMPDIR) work.
//!
//! Two contracts:
//! 1. The self-injected TMPDIR default (parent env has no TMPDIR) is a
//!    shell-only variable — GNU variables.c initialize_shell_variables never
//!    invents a TMPDIR and never exports one it did not import. Foreign
//!    children see NO TMPDIR, exactly like under cmd/PowerShell.
//! 2. A genuinely exported TMPDIR (inherited, or the user exported it) still
//!    reaches children, but in Windows-native forward-slash form — the same
//!    boundary rule as PATH (shell display form inside, process form at the
//!    child boundary; docs/issue329-path-form-policy-analysis.md).

use std::process::Command;

fn rubash(args: &[&str], env_clear: bool) -> (String, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command.args(args);
    if env_clear {
        command.env_remove("TMPDIR");
    }
    let output = command.output().expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// cmd.exe prints an undefined `%VAR%` reference verbatim, so `%TMPDIR%` in
/// the output means the child env block has no TMPDIR; any other value is
/// what the foreign child actually received.
fn cmd_probe(script: &str, env_clear: bool) -> String {
    let (stdout, stderr) = rubash(&["-c", script], env_clear);
    assert!(
        stderr.is_empty(),
        "unexpected stderr for {script:?}: {stderr}"
    );
    stdout.trim_end().to_string()
}

/// The self-injected TMPDIR default must NOT be exported to foreign
/// children: before the fix a POSIX-form `/c/...` TMPDIR reached opencode
/// and broke its bunfs DLL extraction (niubash#162).
#[test]
fn injected_tmpdir_default_is_not_exported_to_children() {
    let out = cmd_probe(r#"cmd /c "echo [%TMPDIR%]""#, true);
    assert_eq!(
        out, "[%TMPDIR%]",
        "foreign child must not see a self-injected TMPDIR"
    );
}

/// The fixture survives as a SHELL variable (suites write unquoted
/// `$TMPDIR/...` paths) — it just is not exported.
#[test]
fn injected_tmpdir_default_still_expands_in_shell() {
    let (stdout, stderr) = rubash(&["-c", "test -n \"$TMPDIR\" && echo SET"], true);
    assert_eq!(stdout.trim_end(), "SET", "stderr: {stderr}");
}

/// `export -p` must not list the injected default (GNU exports only what it
/// imported or was told to export).
#[test]
fn injected_tmpdir_default_is_not_in_export_p() {
    let (stdout, stderr) = rubash(&["-c", "export -p | grep TMPDIR; true"], true);
    assert!(
        !stdout.contains("TMPDIR"),
        "export -p listed TMPDIR: {stdout}, stderr: {stderr}"
    );
}

/// An explicitly exported slash-drive TMPDIR crosses the child boundary in
/// Windows-native form: `C:/...`, not `/c/...`.
#[test]
fn exported_slash_drive_tmpdir_crosses_as_native() {
    let out = cmd_probe(
        r#"export TMPDIR=/c/TempProbeN162; cmd /c "echo %TMPDIR%""#,
        true,
    );
    assert_eq!(out, "C:/TempProbeN162");
}

/// An inherited Windows-form TMPDIR (backslashes, e.g. from a parent that
/// set it natively) stays valid for the child after the boundary pass.
#[test]
fn exported_native_tmpdir_crosses_forward_slash() {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .args(["-c", r#"cmd /c "echo %TMPDIR%""#])
        .env("TMPDIR", r"C:\Users\probe162\tmp");
    let output = command.output().expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(
        stdout.trim_end(),
        "C:/Users/probe162/tmp",
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The `env` builtin boundary keeps the same contract as the direct spawn
/// path.
#[test]
fn env_builtin_tmpdir_boundary_is_native() {
    let out = cmd_probe(
        r#"export TMPDIR=/c/TempProbeN162; env cmd /c "echo %TMPDIR%""#,
        true,
    );
    assert_eq!(out, "C:/TempProbeN162");
}

/// The shell-side display form of an exported TMPDIR stays POSIX
/// (rubash#331 FFmpeg corollary) — only the child boundary is native.
#[test]
fn exported_tmpdir_shell_display_stays_posix() {
    let (stdout, stderr) = rubash(
        &["-c", "export TMPDIR=/c/TempProbeN162; echo $TMPDIR"],
        true,
    );
    assert_eq!(stdout.trim_end(), "/c/TempProbeN162", "stderr: {stderr}");
}
