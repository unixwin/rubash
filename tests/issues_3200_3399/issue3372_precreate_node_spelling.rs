//! rubash#3372 — the redirect pre-create opens resolve a redirect
//! target's byte spelling through the redirect NODE, like the append
//! funnel (#3218) and the create funnel (#3368).
//!
//! `a="$(printf 'x\\234')"` (raw byte 0x9C) and `b=x$'\u009c'` (the
//! genuine char U+009C, UTF-8 C2 9C) fold to the same string. GNU
//! redir.c:298 redirection_expand hands each redirect its own expanded
//! bytes and redir.c:702 redir_open opens them verbatim, so
//! `f >"$a" 2>"$b"` creates two files: `78 9C` and `78 C2 9C`. The
//! pre-create opens (`create_redirect_output_prepared_open_for` in
//! source_type_state.rs and `create_redirect_output_prepared` in
//! function_calls.rs) passed `None` for the node and consulted only
//! the folded-string record, which the first expansion had filled
//! with the raw-byte spelling — so the genuine leg truncated the
//! raw-byte file instead of creating the UTF-8 one. The node-aware
//! `*_for` forms now read the node's own record.
//!
//! Each case runs in its own directory; expectations are the directory
//! listings GNU bash 5.3.20 (LC_ALL=C.UTF-8) leaves for the same script
//! (oracle probes p1a/p1b/p1c, 2026-11-18).
//!
//! NOTE: these pins are written fail-first in form but were NOT run
//! (no build route: X299 tunnel down, local build banned, box wallet
//! empty) — UNVERIFIED-BY-CAPTAIN. The captain verifies them on a live
//! route before merge.
#![cfg(unix)]

use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3372-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Run `body` (with `$a`/`$b` bound) inside a fresh work dir; return the
/// sorted (name bytes, contents) listing of that dir.
fn listing(tag: &str, body: &str) -> Vec<(Vec<u8>, String)> {
    let dir = scratch(tag);
    let work = dir.join("work");
    std::fs::create_dir_all(&work).expect("work dir");
    let script = dir.join("probe.sh");
    std::fs::write(
        &script,
        format!("cd \"$1\" || exit 1\na=\"$(printf 'x\\234')\"\nb=x$'\\u009c'\n{body}\n"),
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
        "stderr for {body:?}"
    );
    let entries = read_listing(&work);
    let _ = std::fs::remove_dir_all(&dir);
    entries
}

fn read_listing(dir: &Path) -> Vec<(Vec<u8>, String)> {
    let mut entries: Vec<(Vec<u8>, String)> = std::fs::read_dir(dir)
        .expect("read dir")
        .map(|entry| {
            let entry = entry.expect("entry");
            let contents = std::fs::read_to_string(entry.path()).expect("read entry");
            (entry.file_name().as_bytes().to_vec(), contents)
        })
        .collect();
    entries.sort();
    entries
}

fn both(raw: &str, genuine: &str) -> Vec<(Vec<u8>, String)> {
    vec![
        (vec![0x78, 0x9C], raw.to_string()),
        (vec![0x78, 0xC2, 0x9C], genuine.to_string()),
    ]
}

#[test]
fn issue3372_function_precreate_fd1_raw_fd2_genuine() {
    assert_eq!(
        listing("fn1", "f() { :; }\nf >\"$a\" 2>\"$b\""),
        both("", "")
    );
}

#[test]
fn issue3372_function_precreate_fd1_genuine_fd2_raw() {
    assert_eq!(
        listing("fn2", "f() { :; }\nf >\"$b\" 2>\"$a\""),
        both("", "")
    );
}

#[test]
fn issue3372_group_precreate_fd1_raw_fd2_genuine() {
    assert_eq!(listing("grp", "{ : >\"$a\" 2>\"$b\"; }"), both("", ""));
}
