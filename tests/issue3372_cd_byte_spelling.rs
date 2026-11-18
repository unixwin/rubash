//! rubash#3372 — `cd` probes a raw-byte directory by its pre-fold byte
//! spelling, not its UTF-8 re-encoding.
//!
//! `a="$(printf 'x\\234')"` (raw byte 0x9C) and `b=x$'\u009c'` (the
//! genuine char U+009C, UTF-8 C2 9C) fold to the same string. GNU
//! builtins/cd.def change_to_directory chdirs the verbatim bytes, so
//! `cd "$a"` enters the `78 9C` directory and `cd "$b"` enters
//! `78 C2 9C`. rubash probed the folded string, so `cd "$a"` entered
//! the UTF-8 directory instead. `cd_filesystem_path` (builtins/cd)
//! now takes the byte spelling computed before the fold
//! (`transport_name_spelling_bytes`) on unix; without transport the
//! folded road is unchanged, so a genuine non-ASCII name keeps its
//! UTF-8 bytes.
//!
//! The pins assert the ENTERING behavior with a marker file in each
//! directory — not `pwd` output: after the fix `pwd` still prints the
//! folded (UTF-8) spelling of a raw-byte directory, because PWD is
//! bound from the logical (folded) display and `pwd` prints Strings.
//! GNU prints the raw byte there; that display face is an open
//! residual, recorded in the lane REPORT, and is deliberately NOT
//! pinned here so the pin does not enshrine the divergence.
//!
//! Each case runs in its own directory; expectations are the directory
//! shapes GNU bash 5.3.20 (LC_ALL=C.UTF-8) leaves for the same script
//! (oracle probes p3cd/p5cd, 2026-11-18).
//!
//! NOTE: these pins are written fail-first in form but were NOT run
//! (no build route: X299 tunnel down, local build banned, box wallet
//! empty) — UNVERIFIED-BY-CAPTAIN. The captain verifies them on a live
//! route before merge.
#![cfg(unix)]

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3372-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Run `body` (with `$a`/`$b` bound) inside a fresh work dir; return the
/// sorted (dir name bytes, sorted (file, contents) inside) listing.
/// `dirs` are the byte-named directories the case needs, created here
/// because an external `mkdir "$a"` would receive the folded argv
/// (external process spellings are a separate face, not this pin's).
fn dir_listing(
    tag: &str,
    body: &str,
    prelude: &str,
    dirs: &[&[u8]],
) -> Vec<(Vec<u8>, Vec<(Vec<u8>, String)>)> {
    let dir = scratch(tag);
    let work = dir.join("work");
    std::fs::create_dir_all(&work).expect("work dir");
    for name in dirs {
        std::fs::create_dir(work.join(OsStr::from_bytes(name))).expect("fixture dir");
    }
    let script = dir.join("probe.sh");
    std::fs::write(&script, format!("cd \"$1\" || exit 1\n{prelude}\n{body}\n"))
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
        "stderr for {body:?}"
    );
    assert!(
        output.status.success(),
        "script rc={} for {body:?}",
        output.status
    );
    let mut entries: Vec<(Vec<u8>, Vec<(Vec<u8>, String)>)> = std::fs::read_dir(&work)
        .expect("read dir")
        .map(|entry| {
            let entry = entry.expect("entry");
            let mut files: Vec<(Vec<u8>, String)> = std::fs::read_dir(entry.path())
                .expect("read subdir")
                .map(|f| {
                    let f = f.expect("file");
                    let contents = std::fs::read_to_string(f.path()).expect("read file");
                    (f.file_name().as_bytes().to_vec(), contents)
                })
                .collect();
            files.sort();
            (entry.file_name().as_bytes().to_vec(), files)
        })
        .collect();
    entries.sort();
    let _ = std::fs::remove_dir_all(&dir);
    entries
}

fn marker(name: &[u8], contents: &str) -> (Vec<u8>, Vec<(Vec<u8>, String)>) {
    (
        name.to_vec(),
        vec![(b"marker".to_vec(), contents.to_string())],
    )
}

const PRELUDE_PLAIN: &str = "a=\"$(printf 'x\\234')\"\nb=x$'\\u009c'";
const PRELUDE_MIXED: &str = "a=\"é$(printf '\\234')\"\nb=\"éx\"$'\\u009c'";

const BODY: &str = "cd \"$a\" || exit 99\nprintf 'RAW-MARKER' > marker\ncd ..\ncd \"$b\" || exit 98\nprintf 'UTF-MARKER' > marker\ncd ..";

#[test]
fn issue3372_cd_enters_raw_byte_directory() {
    assert_eq!(
        dir_listing(
            "cd1",
            BODY,
            PRELUDE_PLAIN,
            &[&[0x78, 0x9C], &[0x78, 0xC2, 0x9C]],
        ),
        vec![
            marker(&[0x78, 0x9C], "RAW-MARKER"),
            marker(&[0x78, 0xC2, 0x9C], "UTF-MARKER"),
        ]
    );
}

#[test]
fn issue3372_cd_enters_mixed_raw_byte_directory() {
    assert_eq!(
        dir_listing(
            "cd2",
            BODY,
            PRELUDE_MIXED,
            &[&[0xC3, 0xA9, 0x9C], &[0xC3, 0xA9, 0x78, 0xC2, 0x9C]],
        ),
        vec![
            marker(&[0xC3, 0xA9, 0x78, 0xC2, 0x9C], "UTF-MARKER"),
            marker(&[0xC3, 0xA9, 0x9C], "RAW-MARKER"),
        ]
    );
}
