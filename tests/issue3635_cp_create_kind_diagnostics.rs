//! rubash#3635 — the emulated `cp`'s failed-create diagnostic names
//! the node kind, matching GNU coreutils copy.c copy_internal:
//! `cannot create regular file '<dst>'` for file targets and
//! `cannot create directory '<dst>'` for the recursive-tree branch.
//!
//! Before this fix both faces rendered a bare `cannot create '<dst>'`,
//! dropping the kind word — the builtins.tests
//! `cp /dev/null /zero-length-file-$$` residual in the run-builtins
//! lane (#2666).
//!
//! Oracle (GNU bash 5.3.20 + coreutils):
//!   cp /dev/null /zero-length-file-x
//!     -> cp: cannot create regular file '/zero-length-file-x':
//!        Permission denied   (rc 1)
//!   cp -r /tmp/…/a /zero-dir-x
//!     -> cp: cannot create directory '/zero-dir-x':
//!        Permission denied   (rc 1)

#![cfg(unix)]

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn run_rubash(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-3635-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    let script_path = dir.join("script.sh");
    std::fs::write(&script_path, script).expect("write script");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.current_dir(&dir)
        .arg(&script_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match child.try_wait().expect("poll rubash") {
            Some(_) => break,
            None => {
                assert!(Instant::now() < deadline, "rubash timed out: {script}");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
    let output = child.wait_with_output().expect("collect rubash output");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code(),
    )
}

#[test]
fn cp_regular_file_create_failure_names_regular_file() {
    let (stdout, stderr, rc) = run_rubash("regular-file", "cp /dev/null /zero-length-file-x3635\n");
    assert_eq!(stdout, "");
    assert_eq!(rc, Some(1));
    assert_eq!(
        stderr,
        "cp: cannot create regular file '/zero-length-file-x3635': Permission denied\n"
    );
}

#[test]
fn cp_directory_create_failure_names_directory() {
    let (stdout, stderr, rc) = run_rubash(
        "directory",
        "mkdir -p /tmp/rb3635-src/a\ncp -r /tmp/rb3635-src/a /zero-dir-x3635\n",
    );
    assert_eq!(stdout, "");
    assert_eq!(rc, Some(1));
    assert_eq!(
        stderr,
        "cp: cannot create directory '/zero-dir-x3635': Permission denied\n"
    );
}
