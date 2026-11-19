//! rubash#3945 — `read -t 0` is GNU's non-destructive readiness check:
//! rc 0 only when the descriptor is readable (`input_avail(fd)`,
//! builtins/read.def:395-398 + lib/sh/input_avail.c — a zero-timeout
//! select), rc 1 when nothing has arrived, and it NEVER consumes input.
//! Rubash answered from materialized-stream state instead: a staged
//! FUNCTION_STDIN mirror reported ready while the producer was still
//! sleeping, an exec-bound fd-0 File endpoint was slurped to EOF (the
//! `read -t 0` blocked for the producer's whole runtime and ate the
//! pending record — the closed issue #398's consumption face), and a
//! process-substitution endpoint answered ready unconditionally.
//!
//! GNU bash 5.3.20 oracle expectations, measured with script files.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn run_script(script: &str) -> (String, String, Option<i32>) {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir: PathBuf = std::env::temp_dir().join(format!("rubash-3945-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(path.to_str().expect("utf8 path"))
        .env("LC_ALL", "C.UTF-8")
        .env("TMPDIR", &dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn t0_regular_file_is_always_readable_and_consumes_nothing() {
    // read.def input_avail: a regular file (even empty, even at EOF) is
    // readable to poll(), so -t 0 succeeds — and consumes nothing.
    let script = "printf 'hello\\n' > f; exec 0< f; read -t 0 x; rc=$?; \
                  read y; printf 'rc=%d y=[%s]\\n' \"$rc\" \"$y\"\n";
    let (stdout, _, code) = run_script(script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0 y=[hello]\n");
}

#[test]
fn t0_devnull_is_readable() {
    let script = "read -t 0 x < /dev/null; echo \"rc=$? x=[$x]\"\n";
    let (stdout, _, code) = run_script(script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0 x=[]\n");
}

#[test]
fn t0_process_substitution_without_data_fails() {
    // The writer sleeps before producing: at -t 0 time the pipe GNU
    // polls holds nothing, so the read fails and consumes nothing. The
    // follow-up read then serves the record that arrived meanwhile.
    let script = "exec 0< <(sleep 1; printf 'x\\n'); read -t 0 x; rc=$?; \
                  read y; printf 'rc=%d y=[%s]\\n' \"$rc\" \"$y\"\n";
    let (stdout, _, code) = run_script(script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1 y=[x]\n");
}

#[test]
fn t0_delimited_process_substitution_without_data_fails() {
    let script = "exec 0< <(sleep 1; printf 'a;b'); read -t 0 -d ';' x; rc=$?; \
                  read -d ';' y; printf 'rc=%d y=[%s]\\n' \"$rc\" \"$y\"\n";
    let (stdout, _, code) = run_script(script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1 y=[a]\n");
}

#[test]
fn t0_herestring_does_not_consume_pending_input() {
    // Closed issue #398's face: -t 0 on the herestring reports the
    // materialized body readable and leaves it for the next read.
    let script = "{ read -t 0 probe; first_status=$?; read value; \
                   printf '<%s>:%s:<%s>' \"$probe\" \"$first_status\" \"$value\"; } <<< abc\n";
    let (stdout, _, code) = run_script(script);
    assert_eq!(code, Some(0));
    // The script's printf has no trailing newline, matching the seed.
    assert_eq!(stdout, "<>:0:<abc>");
}

#[test]
fn t0_pipe_without_ready_data_fails() {
    // A live pipe whose producer has not written yet: GNU's select(0)
    // reports nothing readable -> rc 1.
    let script = "{ sleep 1; printf 'x\\n'; } | { read -t 0 reply; \
                   rc=$?; read -t 5 rest; printf 'rc=%d rest=[%s]\\n' \"$rc\" \"$rest\"; }\n";
    let (stdout, _, code) = run_script(script);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1 rest=[x]\n");
}
