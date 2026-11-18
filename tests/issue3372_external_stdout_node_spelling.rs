//! rubash#3372 — the external stdout target open resolves a redirect
//! target's byte spelling through the redirect NODE, like the create
//! funnel (#3218).
//!
//! `a="$(printf 'x\\234')"` (raw byte 0x9C) and `b=x$'\u009c'` (the
//! genuine char U+009C, UTF-8 C2 9C) fold to the same string. GNU
//! redir.c:298 redirection_expand hands each redirect its own expanded
//! bytes and redir.c:702 redir_open opens them verbatim, so
//! `true >"$b" 2>"$a"` creates two files: `78 9C` and `78 C2 9C`.
//! `apply_external_stdout_target` (external_redirects.rs) opened the
//! create leg through the string-only `create_redirect_output`, which
//! consulted the folded-string record — the sibling `2>"$a"`
//! expansion had filled it with the raw-byte spelling, so the
//! genuine leg created the raw-byte file and the UTF-8 one never
//! appeared. The node-aware `create_redirect_output_for` now reads
//! the node's own record.
//!
//! Each case runs in its own directory; expectations are the directory
//! listings GNU bash 5.3.20 (LC_ALL=C.UTF-8) leaves for the same script
//! (oracle probes p2a/p2b, 2026-11-18).
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
fn issue3372_external_stdout_fd1_genuine_fd2_raw() {
    assert_eq!(listing("ext1", "true >\"$b\" 2>\"$a\""), both("", ""));
}

#[test]
fn issue3372_external_stdout_fd1_raw_fd2_genuine() {
    assert_eq!(listing("ext2", "true >\"$a\" 2>\"$b\""), both("", ""));
}

#[test]
fn issue3372_dupe_output_word_byte_spelling() {
    // `>&word` with a non-numeric word is r_err_and_out (GNU
    // redir.c:832-838): ONE open of `word` shared by fds 1 and 2.
    // The `&` the parser stores in the target is operator syntax
    // (parser/redirections.rs redirect_target), never filename
    // content, so the node's recorded transport spelling must drop
    // it — verbatim use opened a literal `&name` file.
    assert_eq!(
        listing("ext3", "echo RAW >&\"$a\"\necho UTF >&\"$b\""),
        vec![
            (vec![0x78, 0x9C], "RAW\n".to_string()),
            (vec![0x78, 0xC2, 0x9C], "UTF\n".to_string()),
        ]
    );
}
