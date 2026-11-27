//! rubash#1206 regressions: `read -t N` ignored its timeout when stdin
//! was a pipe — `(sleep 2; echo late) | { read -t 1 x; }` waited for the
//! producer and consumed the late line with status 0. The timed-read
//! fast path (pipeline_exec.rs execute_timed_read_pipeline) models
//! producer timing chunk-by-chunk; it now walks `( ... )` subshell bodies
//! exactly like `{ ...; }` brace groups, so a delayed subshell producer
//! expires the deadline and the read reports 128+SIGALRM (142).
//!
//! Expectations verified byte-for-byte against WSL GNU Bash 5.3.20.

use std::process::Command;
use std::time::Instant;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1206-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn read_timeout_expires_before_late_data() {
    let (stdout, _, code) =
        rubash_file("(sleep 2; echo late) | { read -t 1 x; echo \"rc=$? x=[$x]\"; }\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=142 x=[]\n");
}

#[test]
fn read_timeout_expires_on_silent_pipe() {
    let (stdout, _, code) = rubash_file("(sleep 3) | { read -t 1 y; echo \"rc=$? y=[$y]\"; }\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=142 y=[]\n");
}

#[test]
fn read_timeout_returns_promptly() {
    let started = Instant::now();
    let (stdout, _, _) = rubash_file("(sleep 3) | { read -t 1 y; echo \"rc=$? y=[$y]\"; }\n");
    assert_eq!(stdout, "rc=142 y=[]\n");
    // GNU returns ~1s; the producer's 3s must not be paid. Generous bound
    // for loaded CI hosts.
    assert!(started.elapsed().as_secs() < 3);
}

#[test]
fn read_timeout_still_reads_ready_data() {
    let (stdout, _, code) = rubash_file("echo hi | { read -t 2 z; echo \"rc=$? z=[$z]\"; }\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0 z=[hi]\n");
}
