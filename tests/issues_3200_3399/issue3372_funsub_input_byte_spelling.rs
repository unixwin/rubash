//! rubash#3372 — `$(<file)` reads a raw-byte file by its pre-fold byte
//! spelling, not its UTF-8 re-encoding.
//!
//! `a="$(printf 'x\\234')"` (raw byte 0x9C) and `b=x$'\u009c'` (the
//! genuine char U+009C, UTF-8 C2 9C) fold to the same string. GNU
//! subst.c command_substitute reads the verbatim bytes, so
//! `x=$(<"$a")` reads the `78 9C` file and `y=$(<"$b")` reads
//! `78 C2 9C`. rubash computed the path from the folded name
//! (`path_buf_from_folded_name`), which cannot tell a folded byte
//! char from a genuine char of the same code point — a name mixing
//! both (genuine `é` beside a raw byte 0x9C) misspelled the genuine
//! char as its single byte. The read path now takes the byte
//! spelling computed from the pre-fold word
//! (`transport_name_spelling_bytes`) on unix; without transport the
//! folded road is unchanged.
//!
//! Each case runs in its own directory; expectations are the stdout
//! GNU bash 5.3.20 (LC_ALL=C.UTF-8) prints for the same script
//! (oracle probes p3sub/p5sub, 2026-11-18).
//!
//! NOTE: these pins are written fail-first in form but were NOT run
//! (no build route: X299 tunnel down, local build banned, box wallet
//! empty) — UNVERIFIED-BY-CAPTAIN. The captain verifies them on a live
//! route before merge.
#![cfg(unix)]

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
/// raw stdout bytes.
fn stdout_of(tag: &str, body: &str, prelude: &str) -> Vec<u8> {
    let dir = scratch(tag);
    let work = dir.join("work");
    std::fs::create_dir_all(&work).expect("work dir");
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
    let _ = std::fs::remove_dir_all(&dir);
    output.stdout
}

const PRELUDE_PLAIN: &str = "a=\"$(printf 'x\\234')\"\nb=x$'\\u009c'";
const PRELUDE_MIXED: &str = "a=\"é$(printf '\\234')\"\nb=\"éx\"$'\\u009c'";

const BODY_PLAIN: &str = "printf 'RAW' > \"$a\"\nprintf 'UTF' > \"$b\"\nx=$(<\"$a\"); printf 'sub-a=[%s]\\n' \"$x\"\ny=$(<\"$b\"); printf 'sub-b=[%s]\\n' \"$y\"";
const BODY_MIXED: &str = "printf 'MIXED-RAW' > \"$a\"\nprintf 'MIXED-UTF' > \"$b\"\nx=$(<\"$a\"); printf 'sub-a=[%s]\\n' \"$x\"\ny=$(<\"$b\"); printf 'sub-b=[%s]\\n' \"$y\"";

#[test]
fn issue3372_funsub_input_reads_raw_byte_file() {
    assert_eq!(
        stdout_of("sub1", BODY_PLAIN, PRELUDE_PLAIN),
        b"sub-a=[RAW]\nsub-b=[UTF]\n"
    );
}

#[test]
fn issue3372_funsub_input_reads_mixed_raw_byte_file() {
    assert_eq!(
        stdout_of("sub2", BODY_MIXED, PRELUDE_MIXED),
        b"sub-a=[MIXED-RAW]\nsub-b=[MIXED-UTF]\n"
    );
}
