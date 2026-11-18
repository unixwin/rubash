//! rubash#3372 face 3 — `type` / `type -p` / `command -v` / `type -a`
//! print a raw-byte name and its resolved path as bytes, not as the
//! folded string's UTF-8 re-encoding.
//!
//! `a="$(printf 'x\\234')"` (raw byte 0x9C) and `b=x$'\u009c'` (the
//! genuine char U+009C, UTF-8 C2 9C) fold to the same string. GNU
//! builtins/type.def describe_command prints the name and the
//! find_user_command hit verbatim, so `type "$a"` prints
//! `x\x9c is <work>/x\x9c` and `type "$b"` prints
//! `x\xc2\x9c is <work>/x\xc2\x9c`. rubash wrote the describe lines
//! with `writeln!` of the folded string, re-encoding the raw byte as
//! UTF-8. The describe lines are now written byte-wise
//! (`write_describe_line` + `describe_display_bytes`) in both
//! `describe_name_with_io` and `describe_name_all_with_io`; with no
//! transport the byte spelling is the folded name's UTF-8 bytes, so
//! plain names print exactly as before.
//!
//! Each case runs in its own directory; expectations are the stdout
//! GNU bash 5.3.20 (LC_ALL=C.UTF-8) prints for the same script
//! (oracle probe p4type, 2026-11-18).
//!
//! NOTE: these pins are written fail-first in form but were NOT run
//! (no build route: X299 tunnel down, local build banned, box wallet
//! empty) — UNVERIFIED-BY-CAPTAIN. The captain verifies them on a live
//! route before merge.
#![cfg(unix)]

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

const RAW: &[u8] = b"x\x9c";
const GENUINE: &[u8] = b"x\xc2\x9c";

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3372-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Run the type probe inside a fresh work dir; return the raw stdout
/// bytes. The fixture executables are created here rather than by the
/// script: an external `printf >`/`chmod` would receive the folded
/// argv and could not address the raw-byte spelling (external process
/// spellings are a separate face, not this pin's).
fn stdout_of(tag: &str) -> (Vec<u8>, PathBuf) {
    let dir = scratch(tag);
    let work = dir.join("work");
    std::fs::create_dir_all(&work).expect("work dir");
    for name in [RAW, GENUINE] {
        let path = work.join(OsStr::from_bytes(name));
        std::fs::write(&path, b"#!/bin/sh\necho x\n").expect("fixture file");
        let mut perms = std::fs::metadata(&path).expect("meta").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("fixture chmod");
    }
    let script = dir.join("probe.sh");
    std::fs::write(
        &script,
        "a=\"$(printf 'x\\234')\"\nb=x$'\\u009c'\n\
         PATH=\"$1:$PATH\"\n\
         type \"$a\"\ntype \"$b\"\ntype -p \"$a\"\ntype -p \"$b\"\n\
         command -v \"$a\"\ncommand -v \"$b\"\ntype -a \"$a\"\ntype -a \"$b\"\n",
    )
    .expect("write script");
    let output = Command::new(BIN)
        .arg(&script)
        .arg(&work)
        .env("LC_ALL", "C.UTF-8")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "",
        "stderr for type probe"
    );
    assert!(output.status.success(), "script rc={}", output.status);
    (output.stdout, work)
}

fn expected(work: &PathBuf) -> Vec<u8> {
    let w = work.as_os_str().as_bytes();
    let mut out = Vec::new();
    // type "$a" / type "$b"
    out.extend_from_slice(RAW);
    out.extend_from_slice(b" is ");
    out.extend_from_slice(w);
    out.extend_from_slice(b"/");
    out.extend_from_slice(RAW);
    out.extend_from_slice(b"\n");
    out.extend_from_slice(GENUINE);
    out.extend_from_slice(b" is ");
    out.extend_from_slice(w);
    out.extend_from_slice(b"/");
    out.extend_from_slice(GENUINE);
    out.extend_from_slice(b"\n");
    // type -p "$a" / type -p "$b"
    out.extend_from_slice(w);
    out.extend_from_slice(b"/");
    out.extend_from_slice(RAW);
    out.extend_from_slice(b"\n");
    out.extend_from_slice(w);
    out.extend_from_slice(b"/");
    out.extend_from_slice(GENUINE);
    out.extend_from_slice(b"\n");
    // command -v "$a" / command -v "$b"
    out.extend_from_slice(w);
    out.extend_from_slice(b"/");
    out.extend_from_slice(RAW);
    out.extend_from_slice(b"\n");
    out.extend_from_slice(w);
    out.extend_from_slice(b"/");
    out.extend_from_slice(GENUINE);
    out.extend_from_slice(b"\n");
    // type -a "$a" / type -a "$b" (one PATH hit each: same line as type)
    out.extend_from_slice(RAW);
    out.extend_from_slice(b" is ");
    out.extend_from_slice(w);
    out.extend_from_slice(b"/");
    out.extend_from_slice(RAW);
    out.extend_from_slice(b"\n");
    out.extend_from_slice(GENUINE);
    out.extend_from_slice(b" is ");
    out.extend_from_slice(w);
    out.extend_from_slice(b"/");
    out.extend_from_slice(GENUINE);
    out.extend_from_slice(b"\n");
    out
}

#[test]
fn issue3372_type_prints_raw_byte_name_and_path() {
    let (stdout, work) = stdout_of("type1");
    assert_eq!(stdout, expected(&work));
    let _ = std::fs::remove_dir_all(work.parent().expect("scratch parent"));
}
