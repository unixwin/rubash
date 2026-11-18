//! rubash#3372 — the append open resolves a redirect target's byte
//! spelling through the redirect NODE, like the create funnel (#3218).
//!
//! `a="$(printf 'x\234')"` (raw byte 0x9C) and `b=x$'\u009c'` (the
//! genuine char U+009C, UTF-8 C2 9C) fold to the same string. GNU
//! redir.c:298 redirection_expand hands each redirect its own expanded
//! bytes and redir.c:702 redir_open opens them verbatim, so
//! `>"$a" >>"$b"` lands two files: `78 9C` and `78 C2 9C`. rubash's
//! append funnel (`open_append_target`) consulted only the folded-string
//! record, which the `>"$a"` expansion had filled with the raw-byte
//! spelling, so the append reopened `78 9C` and `78 C2 9C` was never
//! created. The node-aware `open_append_target_for` (and the append
//! probe) now read the node's own record, falling back to the plain
//! funnel — never the string-keyed record.
//!
//! Each case runs in its own directory; expectations are the directory
//! listings GNU bash 5.3.20 (LC_ALL=C.UTF-8) leaves for the same script.
//! Faces where the append file is now created but the builtin/compound
//! write still lands on the folded-string binding are tracked in #3372
//! and are not pinned here.
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
fn issue3372_null_command_create_then_append() {
    assert_eq!(listing("null", r#": >"$a" >>"$b""#), both("", ""));
}

#[test]
fn issue3372_external_stdout_create_then_append() {
    assert_eq!(
        listing("ext1", r#"/bin/echo B >"$a" >>"$b""#),
        both("", "B\n")
    );
}

#[test]
fn issue3372_external_stderr_create_then_append() {
    assert_eq!(
        listing("ext2", r#"/bin/echo B 2>"$a" 2>>"$b" >&2"#),
        both("", "B\n")
    );
}

#[test]
fn issue3372_external_combined_append() {
    assert_eq!(
        listing("ext3", r#"/bin/echo B >"$a" &>>"$b""#),
        both("", "B\n")
    );
}

#[test]
fn issue3372_external_append_then_append() {
    assert_eq!(
        listing("ext4", r#"/bin/echo B >>"$a" >>"$b""#),
        both("", "B\n")
    );
}

#[test]
fn issue3372_compound_create_then_append_control() {
    // Already agreed before the fix; kept as a guard for the loop face.
    assert_eq!(
        listing(
            "loop",
            r#"while false; do :; done >"$a" >>"$b"; echo B >>"$b""#
        ),
        both("", "B\n")
    );
}
