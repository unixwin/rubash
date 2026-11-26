//! Regression pins for rubash#3218 — name folding loses per-character
//! source fidelity (residual of #3104 / PR #3122).
//!
//! Face 1: one name mixing a genuine Latin-1 char with a raw-byte
//! transport char. `fold_literal_char_escapes` folds the E000 pair for
//! byte 0x9C to U+009C — indistinguishable from a genuine char of the
//! same code point — and `folded_name_to_path_bytes` then emits EVERY
//! char <= U+00FF as a single byte, so a genuine `é` (U+00E9, UTF-8
//! C3 A9 on disk in GNU) is written as the single byte 0xE9.
//!
//! Face 2: the redirect transport record (`redirect_target_transport_spellings`)
//! is keyed by the FOLDED target string, so two redirects in one
//! command whose targets fold to the same spelling share the flag: a
//! genuine-char target is opened with raw-byte spelling because a
//! different, transport-carrying target folded to the same string.
//!
//! GNU ownership: GNU has no pair and no fold — the expanded word IS
//! the name, and redir.c:298 redirection_expand returns the string
//! do_redirection_internal opens verbatim, byte for byte.
//!
//! All expectations are the literal GNU bash 5.3.20 on-disk byte
//! spellings for the same probe script under LC_ALL=C.UTF-8
//! (oracle-verified).
#![cfg(unix)]

use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3218-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn run_probe(tag: &str) -> (PathBuf, String, Option<i32>) {
    let dir = scratch(tag);
    let work = dir.join("work");
    std::fs::create_dir_all(&work).expect("work dir");
    let script = dir.join("probe.sh");
    std::fs::write(
        &script,
        "cd \"$1\" || exit 1\n\
         n=\"\u{e9}$(printf '\\234')\"\n\
         : >\"$n\"\n\
         a=\"$(printf 'x\\234')\"\n\
         b=x$'\\u009c'\n\
         : >\"$a\" >\"$b\"\n\
         mkdir ctrl\n\
         cd ctrl\n\
         b2=y$'\\u009c'\n\
         : >\"$b2\"\n",
    )
    .expect("write script");
    let output = Command::new(BIN)
        .arg(&script)
        .arg(&work)
        .env("LC_ALL", "C.UTF-8")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    let result = (
        work,
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    result
}

fn entry_bytes(dir: &PathBuf) -> Vec<Vec<u8>> {
    let mut names: Vec<Vec<u8>> = std::fs::read_dir(dir)
        .expect("read dir")
        .map(|entry| entry.expect("entry").file_name().as_bytes().to_vec())
        .collect();
    names.sort();
    names
}

#[test]
fn issue3218_mixed_genuine_and_byte_char_name_keeps_utf8_for_genuine() {
    // Face 1: `é` (genuine, UTF-8 C3 A9) + byte 0x9C (raw) in one name.
    let (work, stderr, code) = run_probe("face1");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    let names = entry_bytes(&work);
    assert!(
        names.contains(&vec![0xC3, 0xA9, 0x9C]),
        "expected on-disk name C3 A9 9C, got {names:?}"
    );
    let _ = std::fs::remove_dir_all(work.parent().expect("scratch root"));
}

#[test]
fn issue3218_redirect_transport_flag_not_shared_across_folded_spellings() {
    // Face 2: transport target x<0x9C raw> and genuine target x<U+009C>
    // fold to the same string; GNU creates BOTH byte spellings.
    let (work, stderr, code) = run_probe("face2");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    let names = entry_bytes(&work);
    assert!(
        names.contains(&vec![b'x', 0x9C]),
        "expected raw-byte name 78 9C, got {names:?}"
    );
    assert!(
        names.contains(&vec![b'x', 0xC2, 0x9C]),
        "expected genuine-char name 78 C2 9C, got {names:?}"
    );
    // Control inside the same run: the genuine-only redirect in ctrl/
    // (no transport sibling) keeps its UTF-8 spelling.
    let ctrl_names = entry_bytes(&work.join("ctrl"));
    assert_eq!(ctrl_names, vec![vec![b'y', 0xC2, 0x9C]]);
    let _ = std::fs::remove_dir_all(work.parent().expect("scratch root"));
}
