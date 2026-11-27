//! niubash#700: `read` must read the descriptor a `/dev/stdin` (or
//! `/dev/fd/N`, `/proc/self/fd/N`) input redirect names, not the Windows
//! console device.
//!
//! GNU redir.c opens `/dev/stdin` — a symlink to `/proc/self/fd/0` — and the
//! kernel resolves that to a dup of fd 0; the builtin then reads fd 0
//! (read.def:294). On Windows `shell_path_to_windows` maps `/dev/stdin` to
//! `CONIN$`, so the read builtin opened the console and blocked forever
//! (`printf 'SIN\n' | niu -c 'read z < /dev/stdin'` never returned), while
//! `/dev/fd/N` resolved to a nonexistent filesystem path and read empty
//! (niubash#701 R3.1). The redirect operand is now resolved to its descriptor
//! and served through the same fd-table/dup machinery as `<&N`.
//!
//! Every case runs under a wall-clock guard: the #700 defect is a hang, so a
//! regression must fail the test instead of blocking CI. Reference semantics
//! verified against WSL GNU bash 5.3.0.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const RUN_LIMIT: Duration = Duration::from_secs(15);

struct Outcome {
    timed_out: bool,
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run(args: &[&str], stdin: &[u8], cwd: Option<&Path>) -> Outcome {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    let mut child = cmd.spawn().expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(stdin)
        .expect("write stdin");
    drop(child.stdin.take());

    // Drain both pipes on threads so a full pipe buffer cannot stall the
    // child while the main thread polls for exit.
    let mut out_pipe = child.stdout.take().expect("piped stdout");
    let mut err_pipe = child.stderr.take().expect("piped stderr");
    let out_reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = std::io::Read::read_to_end(&mut out_pipe, &mut buffer);
        buffer
    });
    let err_reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = std::io::Read::read_to_end(&mut err_pipe, &mut buffer);
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
    let stdout = String::from_utf8_lossy(&out_reader.join().expect("join stdout")).into_owned();
    let stderr = String::from_utf8_lossy(&err_reader.join().expect("join stderr")).into_owned();
    Outcome {
        timed_out,
        code: status.and_then(|status| status.code()),
        stdout,
        stderr,
    }
}

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue700-{tag}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn assert_ok(args: &[&str], stdin: &[u8], cwd: Option<&Path>, expected: &str) {
    let outcome = run(args, stdin, cwd);
    assert!(
        !outcome.timed_out,
        "rubash hung on {args:?} (issue #700 regression)"
    );
    assert_eq!(outcome.stdout, expected, "stderr: {:?}", outcome.stderr);
    assert_eq!(outcome.code, Some(0), "stderr: {:?}", outcome.stderr);
}

/// The issue's primary repro: piped stdin must reach `read < /dev/stdin`
/// (GNU: `got=[SIN]`; rubash pre-fix hung and was killed at the timeout).
#[test]
fn dev_stdin_pipe_reads_stdin() {
    assert_ok(
        &["-c", "read -r z < /dev/stdin; echo \"got=[$z]\""],
        b"SIN\n",
        None,
        "got=[SIN]\n",
    );
}

/// The file-stdin shape: the enclosing compound binds fd 0, the command's own
/// `< /dev/stdin` names that same descriptor.
#[test]
fn dev_stdin_file_reads_bound_file() {
    let dir = scratch_dir("file");
    std::fs::write(dir.join("somefile"), b"SIN\n").expect("write somefile");
    assert_ok(
        &[
            "-c",
            "{ read -r z < /dev/stdin; echo \"got=[$z]\"; } < somefile",
        ],
        b"",
        Some(&dir),
        "got=[SIN]\n",
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// niubash#701 R3.1: `/dev/fd/N` names descriptor N (`exec 6< f`), not a
/// filesystem path. GNU: `[DATA]`.
#[test]
fn dev_fd_n_reads_exec_descriptor() {
    let dir = scratch_dir("fdn");
    std::fs::write(dir.join("f"), b"DATA\n").expect("write f");
    assert_ok(
        &["-c", "exec 6< f; read -r z < /dev/fd/6; echo \"[$z]\""],
        b"",
        Some(&dir),
        "[DATA]\n",
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The `/proc/self/fd/N` spelling names the same descriptor.
#[test]
fn proc_self_fd_n_reads_exec_descriptor() {
    let dir = scratch_dir("procfd");
    std::fs::write(dir.join("f"), b"DATA\n").expect("write f");
    assert_ok(
        &[
            "-c",
            "exec 6< f; read -r z < /proc/self/fd/6; echo \"[$z]\"",
        ],
        b"",
        Some(&dir),
        "[DATA]\n",
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// `/dev/fd/0` is the piped fd 0, exactly like `/dev/stdin`.
#[test]
fn dev_fd_zero_reads_piped_stdin() {
    assert_ok(
        &["-c", "read -r z < /dev/fd/0; echo \"[$z]\""],
        b"SIN\n",
        None,
        "[SIN]\n",
    );
}
