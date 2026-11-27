//! rubash#1373 — `> /dev/fd/1` and `> /dev/stdout` DUPed the target
//! instead of REOPENING it (no truncation, shared offset); GNU reopens
//! the underlying file, so the write lands at offset 0 of a fresh open
//! while the shell's own fd keeps its offset.
//!
//! Observable with stdout as a regular file: the reopened write
//! overwrites the head, and the shell's next write resumes at its own
//! offset, leaving a NUL gap. Probes: clusters/k9/probes/p1373{,b,c}.sh.
//! Expectations verified against GNU bash 5.3.20 (script files,
//! stdout redirected to a file).

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1373-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

/// Run `script` with the child's stdout attached to a fresh file in a
/// temp dir; return (file bytes, stderr, code).
fn run_stdout_file(tag: &str, script_text: &str) -> (Vec<u8>, String, Option<i32>) {
    let dir = tempdir(tag);
    let script = dir.join("probe.sh");
    std::fs::write(&script, script_text).expect("write script");
    let out_path = dir.join("out.txt");
    let out_file = std::fs::File::create(&out_path).expect("create out file");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out_file))
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    let mut bytes = Vec::new();
    std::fs::File::open(&out_path)
        .expect("open out file")
        .read_to_end(&mut bytes)
        .expect("read out file");
    let result = (
        bytes,
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn dev_fd_1_reopens_with_truncation_and_fresh_offset() {
    let (bytes, _stderr, code) =
        run_stdout_file("trunc", "echo first\necho hi > /dev/fd/1\necho after\n");
    assert_eq!(bytes, b"hi\n\0\0\0after\n");
    assert_eq!(code, Some(0));
}

#[test]
fn dev_stdout_reopens() {
    let (bytes, _stderr, code) =
        run_stdout_file("devstdout", "echo hi > /dev/stdout\necho \"rc=$?\"\n");
    assert_eq!(bytes, b"rc=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn dev_fd_1_append_reopens_in_append_mode() {
    let (bytes, _stderr, code) =
        run_stdout_file("append", "echo first\necho hi >> /dev/fd/1\necho after\n");
    assert_eq!(bytes, b"first\nafter\n");
    assert_eq!(code, Some(0));
}
