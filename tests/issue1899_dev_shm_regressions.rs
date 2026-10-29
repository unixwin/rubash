//! Issue rubash#1899 (Windows): `/dev/shm` is the POSIX shared-memory
//! tmpfs — a real writable directory namespace — but the path mapper
//! had no arm for it, so the spelling fell into the `/dev` capability
//! sentinel (`\\.\WINUXSH_UNSUPPORTED_DEVICE`) and every use failed
//! or landed in a literal enclave path instead of a usable directory.
//! The mapper now resolves /dev/shm like its sibling temp namespaces
//! (/var/tmp discipline, rubash#328): a lazily materialized directory
//! below the per-user temp base (<temp>/dev/shm).
//!
//! GNU behavior: /dev/shm is an ordinary writable directory —
//! redirects create files in it, reads serve them back, `test -d`
//! holds.
//!
//! Windows-gated: off Windows the real tmpfs serves the spelling.

#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1899-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn rubash_c(script: &str, dir: &Path) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

#[test]
fn issue1899_dev_shm_is_a_writable_directory() {
    let dir = tempdir("roundtrip");
    let run = rubash_c(
        "test -d /dev/shm && echo is-dir; \
         echo shm-payload-99 > /dev/shm/pin.txt; \
         test -f /dev/shm/pin.txt && echo is-file; \
         cat /dev/shm/pin.txt; \
         echo more >> /dev/shm/pin.txt; cat /dev/shm/pin.txt; \
         rm /dev/shm/pin.txt; test -e /dev/shm/pin.txt || echo removed",
        &dir,
    );
    assert_eq!(run.code, Some(0), "stderr={}", run.stderr);
    for needle in ["is-dir", "is-file", "shm-payload-99", "more", "removed"] {
        assert!(
            run.stdout.contains(needle),
            "missing {needle:?}: stdout={}",
            run.stdout
        );
    }
}
