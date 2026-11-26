//! Issue unixwin/rubash#2195: procsub consumer spawn-time fd state
//!
//! Spawn-time fd state is decided where the child is forked:
//! GNU forks procsub consumers mid-do_redirections (redir.c:237,
//! subst.c:6362) so a consumer inherits the fd table as the
//! redirect entries BEFORE its own left it, and a coproc child
//! applies its construct-level redirects of any fd in the child
//! (execute_cmd.c:2477/4244). These pins run rubash as a
//! subprocess and observe files/stdout only; they were run by
//! executing the test binaries directly (outside the build
//! slot, whose flock fd 9 would leak into the children).

//! Verified against GNU Bash 5.3.20 (script-file probes in the
//! cluster workdir): see REPORT.md for the probe ledger.
//!
//! Harness note: these pins run the shell as a subprocess and
//! assert on FILES the detached consumers write, polled to a
//! deadline — the consumers are async children (subst.c FORK_ASYNC)
//! whose writes race the shell's exit by design. No pin depends on
//! fd 9 or on the test process's own fd table shape.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-fd-family-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
    dir
}

fn rubash_stdin(script: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash stdin");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(script.as_bytes())
        .expect("write stdin");
    drop(child.stdin.take());
    let mut out = child.stdout.take().expect("stdout pipe");
    let mut err = child.stderr.take().expect("stderr pipe");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut out, &mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut err, &mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    });
    let status = child.wait().expect("wait rubash");
    (
        out_thread.join().unwrap_or_default(),
        err_thread.join().unwrap_or_default(),
        status.code(),
    )
}

/// Poll a file until its content equals `expected` or the deadline
/// passes; returns the last observed content.
fn wait_file_content(path: &Path, expected: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut last = String::new();
    while Instant::now() < deadline {
        if let Ok(content) = std::fs::read_to_string(path) {
            last = content;
            if last == expected {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    last
}

// ---------------- #2195 ----------------

#[test]
fn issue2195_dup_after_procsub_spawns_exactly_one_consumer() {
    // `echo hi > >(BODY) 2>&1`: GNU forks ONE consumer; the `2>&1`
    // is a dup onto the substitution pipe, not a second
    // substitution. The body appends its payload plus one RAN
    // marker per spawn, so a phantom second consumer (on an empty
    // carrier) shows up as a second RAN with no payload.
    let dir = test_dir("dup-once");
    let log = dir.join("log");
    let script = format!(
        "echo hi > >({{ cat; echo RAN; }} >> {}) 2>&1\nsleep 0.5\n",
        log.display()
    );
    let (stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let content = wait_file_content(&log, "hi\nRAN\n");
    assert_eq!(content, "hi\nRAN\n", "stdout: {stdout:?}");
}

#[test]
fn issue2195_dup_mirror_consumer_does_not_truncate_payload_file() {
    // Probe p2195b: the group's OUT and ERR both land in the
    // consumer's file under GNU (OUT\nERR\n). A phantom second
    // consumer running the same `cat > c1` body truncates the
    // file after the real consumer wrote it, leaving it empty.
    let dir = test_dir("dup-truncate");
    let c1 = dir.join("c1");
    let script = format!(
        "{{ echo OUT; echo ERR >&2; }} > >(cat > {}) 2>&1\nsleep 0.5\n",
        c1.display()
    );
    let (_stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let content = wait_file_content(&c1, "OUT\nERR\n");
    assert_eq!(content, "OUT\nERR\n");
    // Settle re-check: a phantom second consumer truncates the
    // file at ITS spawn, which can land after the real consumer's
    // write — the content must still be intact after settling.
    std::thread::sleep(std::time::Duration::from_secs(1));
    let settled = std::fs::read_to_string(&c1).unwrap_or_default();
    assert_eq!(settled, "OUT\nERR\n");
}

#[test]
fn issue2195_consumer_fd1_binds_to_prefix_file_redirect() {
    // Probe p2195c: `echo hi >plain > >(wc -c)` — the consumer is
    // forked after fd 1 was rebound to plain, so its count lands
    // IN plain, not on the shell's stdout. The payload itself
    // routes to fd 1's LAST binding (the pipe), so plain holds
    // only the consumer's count (oracle-verified on GNU 5.3.20,
    // including the seeded-append and external-command variants).
    let dir = test_dir("prefix-file");
    let plain = dir.join("plain.txt");
    let script = format!("echo hi >{} > >(wc -c)\nsleep 0.5\n", plain.display());
    let (stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let content = wait_file_content(&plain, "3\n");
    assert_eq!(content, "3\n");
    assert!(
        !stdout.contains('3'),
        "consumer output leaked to shell stdout: {stdout:?}"
    );
}

#[test]
fn issue2195_consumer_after_file_redirect_keeps_ambient_stdout() {
    // Probe p2195d (reverse order): the consumer is forked BEFORE
    // the file redirect, so its fd 1 is the shell's stdout and the
    // count prints there (the payload went to plain via fd 1's
    // later binding, so the consumer reads an empty carrier: 0).
    let dir = test_dir("prefix-none");
    let plain = dir.join("plain.txt");
    let script = format!("echo hi > >(wc -c) >{}\nsleep 0.5\n", plain.display());
    let (stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let content = wait_file_content(&plain, "hi\n");
    assert_eq!(content, "hi\n");
    assert_eq!(stdout, "0\n", "consumer count must reach shell stdout");
}

#[test]
fn issue2195_consumer_inherits_prefix_dup_of_file() {
    // `>o.txt 2>&1` before the substitution: the consumer's fd 2
    // is a dup of the file description fd 1 was bound to at that
    // point, so the body's stderr output lands in o.txt. The
    // payload routes to fd 1's last binding (the pipe) and is
    // consumed by grep, so o.txt holds exactly grep's line
    // (oracle-verified on GNU 5.3.20).
    let dir = test_dir("prefix-dup");
    let out = dir.join("o.txt");
    let script = format!(
        "echo hi >{} 2>&1 > >(grep hi >&2)\nsleep 0.5\n",
        out.display()
    );
    let (stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let content = wait_file_content(&out, "hi\n");
    assert_eq!(content, "hi\n", "stdout: {stdout:?}");
}

#[test]
fn issue2195_consumer_fd2_binds_to_prefix_stderr_file() {
    // A prefix `2>file` binds the consumer's stderr: the body's
    // diagnostic channel lands in the file, not on the shell's
    // stderr.
    let dir = test_dir("prefix-err");
    let err_file = dir.join("e.txt");
    let consumer_out = dir.join("cout");
    let script = format!(
        "echo hi 2>{} > >(cat > {}; echo BODYERR >&2)\nsleep 0.5\n",
        err_file.display(),
        consumer_out.display()
    );
    let (_stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let content = wait_file_content(&err_file, "BODYERR\n");
    assert_eq!(content, "BODYERR\n");
    assert!(
        !stderr.contains("BODYERR"),
        "consumer stderr leaked to shell stderr: {stderr:?}"
    );
    let payload = wait_file_content(&consumer_out, "hi\n");
    assert_eq!(payload, "hi\n");
}

#[test]
fn issue2195_two_consumers_get_distinct_carriers() {
    // `> >(A) > >(B)`: GNU forks one consumer per entry with its
    // own pipe; fd 1 ends bound to the SECOND pipe, so consumer A
    // reads EOF (count 0) and consumer B reads the payload
    // (count 3). The bodies log through their own file redirects
    // so the chained stdout binding (B's fd 1 is A's pipe in GNU)
    // stays out of the observation. One shared carrier, or only
    // one consumer spawned, breaks one of the two counts.
    let dir = test_dir("two-consumers");
    let log_a = dir.join("logA.txt");
    let log_b = dir.join("logB.txt");
    let script = format!(
        "echo hi > >(wc -c >{}) > >(wc -c >{})\nsleep 0.5\n",
        log_a.display(),
        log_b.display()
    );
    let (stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr} stdout: {stdout:?}");
    let a = wait_file_content(&log_a, "0\n");
    assert_eq!(a, "0\n");
    let b = wait_file_content(&log_b, "3\n");
    assert_eq!(b, "3\n");
}

#[test]
fn issue2195_combined_redirect_procsub_single_consumer_control() {
    // Control (already working on master): `&> >(wc -c)` runs ONE
    // consumer receiving both streams. The list-keyed
    // materialization restructure must not fork it twice.
    let script = "echo hi &> >(wc -c)\nsleep 0.5\n";
    let (stdout, stderr, code) = rubash_stdin(script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "3\n");
}
