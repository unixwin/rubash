//! rubash#493 (W5 residual / characterization pin): the `du` trailing-
//! slash report (`du -h src/` printing `src//bin` where GNU normalizes)
//! was filed against rubash, but `du` is NOT a rubash builtin — the
//! engine never serves it; it always resolves through PATH to an
//! external `du` (on Windows, the WinuxCmd applet).
//!
//! Disposition (recorded on the issue): duplicate of
//! unixwin/WinuxCmd#1159 — "the applet lives in WinuxCmd, so the fix
//! will land there". WinuxCmd#1159 is closed/completed, i.e. the
//! trailing-slash normalization was fixed in the applet. What remains
//! pinnable on the rubash side is exactly the boundary this file pins:
//! even with an empty PATH, `du` does not resolve to a builtin, so no
//! rubash-side code path can produce the double-slash output. If anyone
//! ever adds a `du` builtin/simulation to the engine, this pin fails
//! and the GNU normalization behavior must be re-adjudicated then.

use std::process::Command;

fn run_with_empty_path(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env_clear()
        .env("PATH", "/nonexistent-bf-w5")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue493_du_is_not_a_shell_builtin() {
    // With an empty PATH a builtin would still report `builtin`; an
    // external-only name reports nothing and fails.
    let (stdout, stderr, status) = run_with_empty_path("type -t du; echo \"rc=$?\"");
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr:?}");
    assert_eq!(status, Some(0), "stderr: {stderr:?}");
}

#[test]
fn issue493_du_does_not_resolve_without_path() {
    let (stdout, _stderr, status) = run_with_empty_path("command -v du; echo \"rc=$?\"");
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(status, Some(0));
}
