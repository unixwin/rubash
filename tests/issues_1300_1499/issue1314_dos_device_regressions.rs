//! Regression pins for rubash#1314 — DOS device path residual edges left
//! over from the niubash#519 family (FILE_APPEND_DATA on device objects).
//!
//! Items pinned (issue body):
//!   1. `HISTFILE` pointing at a DOS device name: history's append open
//!      used `.create(true).append(true)`, which requests
//!      FILE_APPEND_DATA — console device objects reject it (Win32 error
//!      87), so `history -a`/`-w` against a device HISTFILE failed. The
//!      fix aligned every std-open site for device targets to the
//!      plain-write policy already proven for redirects (rubash#1324):
//!      devices are opened write/non-append at the OS layer.
//!   2. `<>CONIN$` / `<>CONOUT$` literal targets: the read/write open
//!      itself must succeed, and the WRONG-direction operation must fail
//!      at use time (GNU shape: open succeeds, operation fails) — the
//!      open splits into a read/write pair (FileFd::open_readwrite_pair
//!      CON handling) instead of one bidirectional handle the console
//!      driver refuses.
//!   3. `open_output_fd_append` opening NUL with write+append: recorded
//!      inconsistency only (NUL accepts FILE_APPEND_DATA, no live fault)
//!      — no pin; noted here so the audit trail survives.
//!
//! These forms are Windows-console-only by construction. Like the
//! niubash#519 pin file, every test self-skips unless the process runs
//! on Windows WITH a live console (headless Windows has no console
//! device either, and /dev/tty names mean something else on unix).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

/// A live console exists iff CON can be opened for writing (same gate
/// as the niubash#519 pin file).
fn console_available() -> bool {
    if !cfg!(windows) {
        return false;
    }
    std::fs::OpenOptions::new().write(true).open("CON").is_ok()
}

#[test]
fn issue1314_histfile_on_device_accepts_append_writes() {
    if !console_available() {
        return;
    }
    // history -a appends the in-memory entries to HISTFILE. With
    // HISTFILE=/dev/tty (a DOS device target), the append-shaped open
    // used to die with Win32 error 87 before writing anything.
    let (stdout, stderr, _code) =
        rubash("HISTFILE=/dev/tty; history -s 'bf-d-1314-marker'; history -a; echo hist_rc=$?");
    assert!(
        stdout.contains("hist_rc=0"),
        "history -a against a device HISTFILE must succeed, stdout: {stdout:?} stderr: {stderr:?}"
    );
    assert!(
        !stderr.contains("87") && !stderr.to_lowercase().contains("invalid"),
        "no FILE_APPEND_DATA error may surface, stderr: {stderr:?}"
    );
}

#[test]
fn issue1314_conin_readwrite_open_succeeds_write_fails_at_use() {
    if !console_available() {
        return;
    }
    // GNU shape: the <> open itself succeeds; the wrong-direction write
    // fails when attempted. (Reading CONIN$ would block on console
    // input, so the failing direction is the write side.)
    let (stdout, stderr, _code) =
        rubash("exec 3<>CONIN$; echo open_rc=$?; echo x >&3 2>/dev/null; echo write_rc=$?");
    assert!(
        stdout.contains("open_rc=0"),
        "<>CONIN$ open must succeed, stdout: {stdout:?} stderr: {stderr:?}"
    );
    assert!(
        !stdout.contains("write_rc=0"),
        "writing to a CONIN$ fd must fail at use time, stdout: {stdout:?}"
    );
}

#[test]
fn issue1314_conout_readwrite_open_succeeds_read_fails_at_use() {
    if !console_available() {
        return;
    }
    // Mirror leg: <>CONOUT$ opens fine; reading from it fails at use
    // (immediately — a console output handle never yields input, so this
    // cannot block the way a CONIN$ read could).
    let (stdout, stderr, _code) =
        rubash("exec 3<>CONOUT$; echo open_rc=$?; read -t 1 line <&3 2>/dev/null; echo read_rc=$?");
    assert!(
        stdout.contains("open_rc=0"),
        "<>CONOUT$ open must succeed, stdout: {stdout:?} stderr: {stderr:?}"
    );
    assert!(
        !stdout.contains("read_rc=0"),
        "reading from a CONOUT$ fd must fail at use time, stdout: {stdout:?}"
    );
}
