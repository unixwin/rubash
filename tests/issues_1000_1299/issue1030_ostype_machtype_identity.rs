//! rubash#1030 (W5 residual / characterization pin): `OSTYPE`/`MACHTYPE`
//! naming. The original report was Android-only: niu reported
//! `OSTYPE=android` / `MACHTYPE=aarch64-unknown-android` where GNU bash
//! 5.3 on Android reports `linux-android` /
//! `aarch64-unknown-linux-android`, so scripts gating on `linux*` took
//! the wrong branch.
//!
//! Current state (verified in src/executor/identity.rs): the engine now
//! derives both values from the GNU configure triple on every platform —
//! on Android it returns the GNU forms above (pinned there by the
//! `android_reports_gnu_linux_android_identity` unit test in identity.rs,
//! `#[cfg(target_os = "android")]`, which cannot run on this host). This
//! file pins the desktop Linux face of the same contract: on a
//! linux-gnu host the values are exactly the glibc GNU oracle's
//! (`OSTYPE=linux-gnu`, `MACHTYPE=<machine>-pc-linux-gnu`), and in
//! particular `OSTYPE` starts with `linux`, the property #1030's
//! affected scripts gate on. Windows personas (msys/windows) are pinned
//! by identity.rs's own unit tests.
#![cfg(all(target_os = "linux", target_env = "gnu"))]

use std::process::Command;

fn run(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// GNU `machine()` spelling of the host arch (identity.rs::machine).
fn gnu_machine() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        "x86" => "i686",
        other => other,
    }
}

#[test]
fn issue1030_linux_gnu_ostype_is_linux_gnu() {
    let (stdout, stderr, status) = run(r#"printf '<%s>\n' "$OSTYPE""#);
    assert_eq!(stdout, "<linux-gnu>\n", "stderr: {stderr:?}");
    assert_eq!(status, Some(0), "stderr: {stderr:?}");
}

#[test]
fn issue1030_linux_gnu_machtype_is_configure_triple() {
    let (stdout, stderr, status) = run(r#"printf '<%s>\n' "$MACHTYPE""#);
    let expected = format!("<{}-pc-linux-gnu>\n", gnu_machine());
    assert_eq!(stdout, expected, "stderr: {stderr:?}");
    assert_eq!(status, Some(0), "stderr: {stderr:?}");
}

#[test]
fn issue1030_ostype_matches_linux_glob_gate() {
    // The script-level property the issue was about: `case $OSTYPE in
    // linux*)` must take the Linux branch on GNU-shaped hosts.
    let (stdout, stderr, status) =
        run(r#"case $OSTYPE in linux*) echo linux-branch;; *) echo other-branch;; esac"#);
    assert_eq!(stdout, "linux-branch\n", "stderr: {stderr:?}");
    assert_eq!(status, Some(0), "stderr: {stderr:?}");
}
