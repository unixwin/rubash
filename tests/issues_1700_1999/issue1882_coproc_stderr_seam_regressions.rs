//! rubash#1882: the paired coproc stderr seam.
//!
//! Face 1 — the coproc child's stderr used to travel a pipe plus a
//! forwarding thread in the parent: when the parent shell exited, the
//! thread died with it and the child's late diagnostics were lost
//! wholesale (GNU's forked child holds the resolved fd-2 endpoint as a
//! real descriptor, so its writes land whenever they happen). The child
//! now receives the handle-backed endpoint directly at spawn (GNU
//! execute_cmd.c make_child model); only the process-substitution
//! carrier keeps the forwarding transport.
//!
//! Face 2 — a coproc child skipped the startup signal-policy capture
//! (its TERM-mailbox exemption in Executor::new), so the exec image's
//! runtime SIG_IGN for SIGPIPE was never reset to the #1333 SIG_DFL
//! policy. A body write to the dead coproc pipe then surfaced as a
//! reported-and-continued `Broken pipe` io error (via
//! report_io_error_routed) where GNU's child dies silently on SIGPIPE
//! mid-body. The child now runs capture_entry_policy_without_mailbox.
//!
//! Every expectation below is GNU bash 5.3.20 script-file behavior,
//! verified byte-for-byte against the oracle before this file was
//! written (cluster probes p1-p7/p12-p18).

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const RUN_LIMIT: Duration = Duration::from_secs(20);

struct RunOutcome {
    timed_out: bool,
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// Run a script file under the built rubash, draining stdout/stderr to
/// EOF (an orphaned coproc child holding an inherited endpoint keeps
/// the harness pipe open until it exits — that is the face-1 delivery
/// channel, so EOF is the correct completion signal).
fn run_rubash_script(dir: &std::path::Path, name: &str, script: &str) -> RunOutcome {
    let script_path = dir.join(name);
    let mut file = std::fs::File::create(&script_path).expect("create probe script");
    file.write_all(script.as_bytes())
        .expect("write probe script");
    drop(file);

    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .spawn()
        .expect("spawn rubash");

    let stdout_pipe = child.stdout.take().expect("piped stdout");
    let stderr_pipe = child.stderr.take().expect("piped stderr");
    let stdout_reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let mut pipe = stdout_pipe;
        let _ = std::io::Read::read_to_end(&mut pipe, &mut buffer);
        buffer
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let mut pipe = stderr_pipe;
        let _ = std::io::Read::read_to_end(&mut pipe, &mut buffer);
        buffer
    });

    let started = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait().expect("poll rubash") {
            Some(status) => break Some(status),
            None => {
                if started.elapsed() >= RUN_LIMIT {
                    timed_out = true;
                    let _ = child.kill();
                    break child.wait().ok();
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    // The drains end at EOF: an orphaned coproc child holding an
    // inherited endpoint keeps the pipe open until it exits, which is
    // exactly the delivery window these pins measure.
    let stdout = stdout_reader.join().expect("join stdout reader");
    let stderr = stderr_reader.join().expect("join stderr reader");
    RunOutcome {
        timed_out,
        code: status.and_then(|status| status.code()),
        stdout,
        stderr,
    }
}

fn scratch(test: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue1882-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

#[test]
fn coproc_child_stderr_survives_parent_exit_simple_command() {
    let dir = scratch("simple");
    let outcome = run_rubash_script(
        &dir,
        "p1.sh",
        "coproc sh -c 'sleep 0.5; echo CHILD-ALIVE >&2'\necho parent-exiting\n",
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!outcome.timed_out);
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, b"parent-exiting\n");
    assert_eq!(outcome.stderr, b"CHILD-ALIVE\n");
}

#[test]
fn coproc_child_stderr_survives_parent_exit_group_body() {
    let dir = scratch("group");
    let outcome = run_rubash_script(
        &dir,
        "p2.sh",
        "coproc G { sleep 0.5; echo G-ALIVE >&2; }\necho parent-exiting\n",
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!outcome.timed_out);
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, b"parent-exiting\n");
    assert_eq!(outcome.stderr, b"G-ALIVE\n");
}

#[test]
fn coproc_child_broken_stdout_pipe_dies_silently_err_first() {
    // The stderr write lands first; the following stdout write meets a
    // pipe whose read end the exited parent took with it. GNU: the
    // child dies on SIGPIPE at that write — silently, mid-body.
    let dir = scratch("errfirst");
    let outcome = run_rubash_script(
        &dir,
        "p4.sh",
        "coproc G { sleep 0.5; echo late-err >&2; echo late-out; }\necho parent-exiting\n",
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!outcome.timed_out);
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, b"parent-exiting\n");
    assert_eq!(outcome.stderr, b"late-err\n");
}

#[test]
fn coproc_child_broken_stdout_pipe_dies_silently_out_first() {
    // The stdout write comes first: GNU's child dies there, so the
    // later stderr line is never even attempted — empty stderr, and in
    // particular no `Broken pipe` self-diagnostic.
    let dir = scratch("outfirst");
    let outcome = run_rubash_script(
        &dir,
        "p3.sh",
        "coproc G { sleep 0.5; echo late-out; echo late-err >&2; }\necho parent-exiting\n",
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!outcome.timed_out);
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, b"parent-exiting\n");
    assert!(outcome.stderr.is_empty(), "stderr: {:?}", outcome.stderr);
}

#[test]
fn coproc_child_epipe_parent_alive_read_end_closed() {
    // Parent stays alive but closes its coproc read fd: same silent
    // death — err1 delivered, the out1 write kills the child, err2 is
    // never reached, and nothing resembling a pipe diagnostic appears.
    let dir = scratch("parentalive");
    let outcome = run_rubash_script(
        &dir,
        "p5.sh",
        "coproc G { sleep 0.3; echo err1 >&2; echo out1; echo err2 >&2; }\n\
         eval \"exec ${G[0]}<&-\"\nsleep 0.8\necho parent-done\n",
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!outcome.timed_out);
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, b"parent-done\n");
    assert_eq!(outcome.stderr, b"err1\n");
}

#[test]
fn coproc_sleep_body_keeps_issue387_surfaces_clean() {
    // The #387 pin shape with the race made deterministic: a slow body
    // whose single stdout write races the parent's exit. GNU prints
    // `pid:set` only, with empty stderr — the write either lands in the
    // still-open pipe or kills the child silently.
    let dir = scratch("clean387");
    let outcome = run_rubash_script(
        &dir,
        "p6.sh",
        "coproc GREP { sleep 0.4; echo co-value; }\necho \"pid:${GREP_PID:+set}\"\n",
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!outcome.timed_out);
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, b"pid:set\n");
    assert!(outcome.stderr.is_empty(), "stderr: {:?}", outcome.stderr);
}

#[test]
fn coproc_stderr_bound_to_file_survives_parent_exit() {
    // `exec 2>file` first: the child inherits the shell's open file
    // description, so its late diagnostic lands in the file even
    // though the parent has exited.
    let dir = scratch("execfile");
    let outcome = run_rubash_script(
        &dir,
        "p13.sh",
        "exec 2>cap.txt\ncoproc G { sleep 0.4; echo SHARED >&2; }\necho done-out\n",
    );
    // The file has no EOF semantics: the shell exits at once, the
    // orphaned child writes at +0.4s. Poll for the delivery with a
    // bounded wait instead of racing the child's sleep.
    let cap_path = dir.join("cap.txt");
    let cap_deadline = Instant::now() + Duration::from_secs(5);
    let mut cap = std::fs::read(&cap_path).unwrap_or_default();
    while cap != b"SHARED\n" && Instant::now() < cap_deadline {
        std::thread::sleep(Duration::from_millis(25));
        cap = std::fs::read(&cap_path).unwrap_or_default();
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!outcome.timed_out);
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, b"done-out\n");
    assert!(outcome.stderr.is_empty(), "stderr: {:?}", outcome.stderr);
    assert_eq!(cap, b"SHARED\n");
}

#[test]
fn coproc_stderr_dup_to_stdout_survives_parent_exit() {
    // `exec 2>&1`: fd 2 resolves to the process-stdout endpoint; the
    // child's late stderr write must surface on stdout after the
    // parent exits.
    let dir = scratch("execdup");
    let outcome = run_rubash_script(
        &dir,
        "p14.sh",
        "exec 2>&1\ncoproc G { sleep 0.4; echo TO-STDOUT >&2; }\necho plain\n",
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!outcome.timed_out);
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, b"plain\nTO-STDOUT\n");
    assert!(outcome.stderr.is_empty(), "stderr: {:?}", outcome.stderr);
}

#[test]
fn coproc_stderr_into_other_coproc_stdin() {
    // The shell's fd 2 bound to another coproc's stdin pipe: the child
    // holds a dup of that pipe write end directly, and the byte is
    // readable from the first coproc.
    let dir = scratch("tocoproc");
    let outcome = run_rubash_script(
        &dir,
        "p15.sh",
        "coproc A { cat; }\nexec 2>&${A[1]}\ncoproc B { echo via-b >&2; }\n\
         read -t 3 line <&${A[0]}\necho \"got: $line\"\n",
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!outcome.timed_out);
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, b"got: via-b\n");
    assert!(outcome.stderr.is_empty(), "stderr: {:?}", outcome.stderr);
}

#[test]
fn coproc_own_stderr_redirect_still_applies() {
    // The coproc command's own `2>file` overrides the inherited
    // endpoint, exactly as before the transport change.
    let dir = scratch("ownredir");
    let outcome = run_rubash_script(
        &dir,
        "p12.sh",
        "coproc G { echo bodyerr >&2; echo bodyout; } 2>cap.txt\n\
         wait $G_PID\necho \"cap: $(cat cap.txt)\"\n",
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!outcome.timed_out);
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, b"cap: bodyerr\n");
    assert!(outcome.stderr.is_empty(), "stderr: {:?}", outcome.stderr);
}
