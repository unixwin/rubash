//! Regression pins for rubash#2736: an in-process fast-path execution
//! (sleep / dirname / basename) must leave the same hash-table
//! resolution record a GNU spawn would.
//!
//! GNU: a command located through PATH for execution is inserted
//! into the hash table at locate time (findcmd.c:416-426
//! `find_user_command` -> `phash_insert`, found=1; `phash_search`
//! bumps times_found, hashlib.c:254), so after `sleep 0` the shell
//! reports `sleep is hashed (/usr/bin/sleep)` and `hash` lists it.
//! The EmulatedExternal layer (cat & co.) already records this way
//! (external_file_builtins' record_emulated_command_resolution);
//! the Fastpath-class faces never recorded, so `type sleep` claimed
//! the PATH body while `hash` had never heard of it.
//!
//! Unix-only: the shapes resolve the ambient /usr/bin bodies.

#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn shell_bin() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_rubash") {
        return PathBuf::from(path);
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(profile)
        .join("rubash")
}

struct Output {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run(dir: &Path, path: &str, script: &str) -> Output {
    let output = Command::new(shell_bin())
        .args(["-c", script])
        .current_dir(dir)
        .env("PATH", path)
        .env_remove("BASH_ENV")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    Output {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).replace('\r', ""),
        code: output.status.code(),
    }
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-2736-{}-{}", tag, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn ambient_path() -> String {
    std::env::var("PATH").unwrap_or_default()
}

/// The hits count `hash` reports for `name`, or None when the name
/// has no hash-table entry. Hash lines are `<hits>\t<full path>`.
fn hash_hits(stdout: &str, name: &str) -> Option<u32> {
    for line in stdout.lines() {
        let mut parts = line.splitn(2, '\t');
        let hits: u32 = parts.next()?.trim().parse().ok()?;
        let path = parts.next()?;
        if path.rsplit('/').next() == Some(name) {
            return Some(hits);
        }
    }
    None
}

#[test]
fn sleep_type_reports_hashed_after_fastpath_run() {
    let dir = scratch("type");
    let out = run(&dir, &ambient_path(), "sleep 0; type sleep");
    assert!(
        out.stdout.contains("sleep is hashed ("),
        "stdout: {}\nstderr: {}",
        out.stdout,
        out.stderr
    );
}

#[test]
fn sleep_and_cat_both_listed_with_one_hit() {
    let dir = scratch("both");
    let out = run(
        &dir,
        &ambient_path(),
        "echo x > f; sleep 0; cat f >/dev/null; hash",
    );
    assert_eq!(hash_hits(&out.stdout, "sleep"), Some(1), "{}", out.stdout);
    assert_eq!(hash_hits(&out.stdout, "cat"), Some(1), "{}", out.stdout);
}

#[test]
fn dirname_basename_recorded() {
    let dir = scratch("dirbase");
    let out = run(
        &dir,
        &ambient_path(),
        "dirname /a/b >/dev/null; basename /a/b >/dev/null; hash",
    );
    assert_eq!(hash_hits(&out.stdout, "dirname"), Some(1), "{}", out.stdout);
    assert_eq!(
        hash_hits(&out.stdout, "basename"),
        Some(1),
        "{}",
        out.stdout
    );
}

#[test]
fn repeated_sleep_bumps_hits() {
    let dir = scratch("repeat");
    let out = run(&dir, &ambient_path(), "sleep 0; sleep 0; hash");
    assert_eq!(hash_hits(&out.stdout, "sleep"), Some(2), "{}", out.stdout);
}

#[test]
fn type_lookup_bumps_recorded_hits_like_gnu() {
    // GNU phash_search bumps times_found, so the `type` lookup itself
    // is the second hit — the same parity the recorded cat entry
    // already shows.
    let dir = scratch("typebump");
    let out = run(
        &dir,
        &ambient_path(),
        "sleep 0; type sleep >/dev/null; hash",
    );
    assert_eq!(hash_hits(&out.stdout, "sleep"), Some(2), "{}", out.stdout);
}

#[test]
fn command_face_dirname_recorded() {
    let dir = scratch("cmdface");
    let out = run(
        &dir,
        &ambient_path(),
        "command dirname /a/b >/dev/null; hash",
    );
    assert_eq!(hash_hits(&out.stdout, "dirname"), Some(1), "{}", out.stdout);
}

#[test]
fn hashall_off_records_nothing() {
    let dir = scratch("nohashall");
    let out = run(&dir, &ambient_path(), "set +H; sleep 0; hash");
    assert_eq!(hash_hits(&out.stdout, "sleep"), None, "{}", out.stdout);
}

#[test]
fn unresolvable_sleep_records_nothing() {
    let dir = scratch("unresolvable");
    let empty = scratch("emptybin");
    let out = run(&dir, empty.to_str().unwrap(), "sleep 0; echo rc=$?; hash");
    assert!(out.stdout.contains("rc=127"), "{}", out.stdout);
    assert_eq!(hash_hits(&out.stdout, "sleep"), None, "{}", out.stdout);
}

#[test]
fn plain_external_record_unchanged() {
    // Control: a plain external command records exactly as before.
    let dir = scratch("control");
    let out = run(
        &dir,
        &ambient_path(),
        "echo hello > f; grep hello f >/dev/null; hash",
    );
    assert_eq!(hash_hits(&out.stdout, "grep"), Some(1), "{}", out.stdout);
}
