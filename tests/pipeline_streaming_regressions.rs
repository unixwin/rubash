//! niubash D4 regression: a pipeline's last member withheld its stdout until
//! EVERY member exited — the niubash#158 anti-deadlock drain slurped the
//! payload into a `Vec` and the deferred `write_pipeline_output` ran only
//! after all waits, so `A | B` emitted everything at once. GNU leaves the
//! rightmost element's fd 1 on the shell's own stdout (execute_cmd.c:2620),
//! so the drain now forwards host-bound bytes live while every capturing
//! shape — `$(...)`, `>f`, `exec 1>file` — keeps the collecting path.
//!
//! Timing canary: `ping -n N 127.0.0.1` emits its first line immediately and
//! then one line per second, so a live pipeline shows its first byte well
//! before a >=2s child exits; the retired withhold-to-exit behaviour cannot
//! produce a byte before the producer's last ping. Windows-only (`ping`,
//! `cmd`), same gate as issue144/issue158.

#![cfg(windows)]

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// First byte must land before this bound on a child that runs >=2s —
/// well clear of both the immediate first ping line and the ~2-3s
/// withhold-to-exit shape.
const LIVE_BOUND: Duration = Duration::from_millis(1500);

struct RunOutcome {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    code: Option<i32>,
}

fn rubash_c(script: &str) -> RunOutcome {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash -c");
    RunOutcome {
        stdout: output.stdout,
        stderr: output.stderr,
        code: output.status.code(),
    }
}

/// Runs a script with piped stdout, recording when the first byte arrived
/// and when EOF hit (the child's exit), plus the exit code.
fn streamed(script: &str) -> (Vec<u8>, Duration, Duration, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn rubash");
    let start = Instant::now();
    let mut stdout = child.stdout.take().expect("piped stdout");
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 4096];
        let mut first = None;
        loop {
            match stdout.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if first.is_none() {
                        first = Some(Instant::now());
                    }
                    bytes.extend_from_slice(&chunk[..n]);
                }
            }
        }
        (bytes, first)
    });
    let (bytes, first) = reader.join().expect("reader thread");
    let eof_at = Instant::now();
    let status = child.wait().expect("wait rubash");
    let ttfb = first
        .expect("streamed pipeline produced no output at all")
        .duration_since(start);
    (bytes, ttfb, eof_at.duration_since(start), status.code())
}

/// Resolve an external tool to an absolute, quoteable script word so the
/// probe cannot fall into rubash's builtin/emulation routing (issue158
/// convention): the drain under test only exists on the OS-pipe spawn path.
fn resolve_tool(name: &str) -> String {
    let with_exe = format!("{name}.exe");
    for dir in std::env::var("PATH")
        .unwrap_or_default()
        .split(';')
        .filter(|entry| !entry.is_empty())
    {
        for candidate in [format!("{dir}\\{with_exe}"), format!("{dir}\\{name}")] {
            let path = PathBuf::from(&candidate);
            if path.is_file() {
                return path.to_string_lossy().replace('\\', "/");
            }
        }
    }
    panic!("D4 tests need a real external `{name}` on PATH");
}

/// Per-test temp directory; removed on drop.
struct Fixture {
    dir: std::path::PathBuf,
}

fn fixture(tag: &str) -> Fixture {
    let dir = std::env::temp_dir().join(format!("rubash-d4-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create d4 fixture dir");
    Fixture { dir }
}

impl Fixture {
    fn script(&self, body: &str) -> String {
        format!(
            "cd '{}' || exit 1\n{}\n",
            self.dir.to_string_lossy().replace('\\', "/"),
            body
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn external_pipeline_first_bytes_arrive_while_members_still_run() {
    let ping = resolve_tool("ping");
    let cat = resolve_tool("cat");
    // `ping -n 4` runs ~3s with its first line immediate; the retired
    // withhold-to-exit drain produced nothing before ~3s.
    let (bytes, ttfb, total, code) = streamed(&format!("\"{ping}\" -n 4 127.0.0.1 | \"{cat}\"\n"));
    assert_eq!(code, Some(0), "pipeline status");
    assert!(
        ttfb < LIVE_BOUND,
        "first pipeline byte withheld {ttfb:?} (niubash D4: drain must forward live)"
    );
    assert!(
        total >= Duration::from_millis(2500),
        "ping -n 4 shape broken: pipeline finished in {total:?}"
    );
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("127.0.0.1"),
        "producer output missing: {text:?}"
    );
}

#[test]
fn streaming_pipeline_does_not_delay_following_statement() {
    let ping = resolve_tool("ping");
    let cat = resolve_tool("cat");
    // Probe shape `A | B; c`: the trailing `echo` must not wait on withheld
    // pipeline bytes, and the pipeline bytes themselves must still be live.
    let (bytes, ttfb, total, code) = streamed(&format!(
        "\"{ping}\" -n 3 127.0.0.1 | \"{cat}\"; echo DONE\n"
    ));
    assert_eq!(code, Some(0), "status");
    assert!(ttfb < LIVE_BOUND, "first byte withheld {ttfb:?}");
    assert!(
        total >= Duration::from_millis(1500),
        "ping -n 3 shape broken: finished in {total:?}"
    );
    let text = String::from_utf8_lossy(&bytes).into_owned();
    assert!(
        text.trim_end().ends_with("DONE"),
        "trailing statement missing from stream: {text:?}"
    );
}

#[test]
fn command_substitution_still_collects_pipeline_output() {
    let ping = resolve_tool("ping");
    let cat = resolve_tool("cat");
    // Capture shape must keep the collecting drain: a stray forward would
    // leak the payload to the host stdout instead of into $x.
    let outcome = rubash_c(&format!(
        "x=$(\"{ping}\" -n 2 127.0.0.1 | \"{cat}\"); echo \"LEN=${{#x}}\"\n"
    ));
    assert_eq!(outcome.code, Some(0), "status");
    let stdout = String::from_utf8_lossy(&outcome.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&outcome.stderr).into_owned();
    let len: usize = stdout
        .trim()
        .strip_prefix("LEN=")
        .and_then(|rest| rest.parse().ok())
        .unwrap_or(0);
    assert!(
        len > 0,
        "substitution captured nothing (stdout={stdout:?}, stderr={stderr:?})"
    );
    assert!(
        !stdout.contains("127.0.0.1"),
        "pipeline output leaked past the substitution capture: {stdout:?}"
    );
    assert_eq!(
        stdout.lines().count(),
        1,
        "host stdout must carry only the length line: {stdout:?}"
    );
}

#[test]
fn output_redirect_tail_still_receives_payload() {
    let fixture = fixture("redir");
    let ping = resolve_tool("ping");
    let cat = resolve_tool("cat");
    let outcome = rubash_c(&fixture.script(&format!(
        "\"{ping}\" -n 2 127.0.0.1 | \"{cat}\" > out.txt\n"
    )));
    assert_eq!(outcome.code, Some(0), "status");
    assert!(
        outcome.stdout.is_empty(),
        "`>f` tail must not stream past the redirect: {:?}",
        String::from_utf8_lossy(&outcome.stdout)
    );
    let written =
        String::from_utf8_lossy(&std::fs::read(fixture.dir.join("out.txt")).unwrap_or_default())
            .into_owned();
    assert!(
        written.contains("127.0.0.1"),
        "redirected tail missing payload: {written:?}"
    );
}

#[test]
fn ambient_fd1_redirect_collects_into_file() {
    let fixture = fixture("exec");
    let ping = resolve_tool("ping");
    let cat = resolve_tool("cat");
    let outcome = rubash_c(&fixture.script(&format!(
        "exec 1>live.txt\n\"{ping}\" -n 2 127.0.0.1 | \"{cat}\"\n"
    )));
    assert_eq!(outcome.code, Some(0), "status");
    assert!(
        outcome.stdout.is_empty(),
        "`exec 1>file` pipeline leaked to the host stdout: {:?}",
        String::from_utf8_lossy(&outcome.stdout)
    );
    let written =
        String::from_utf8_lossy(&std::fs::read(fixture.dir.join("live.txt")).unwrap_or_default())
            .into_owned();
    assert!(
        written.contains("127.0.0.1"),
        "fd-bound pipeline output missing from file: {written:?}"
    );
}

#[test]
fn shell_writer_pipeline_streams_consumer_bytes_live() {
    let ping = resolve_tool("ping");
    // Compound producer finishes immediately; the external consumer keeps
    // running for ~2s. The deferred write used to withhold its output until
    // that consumer exited.
    let (bytes, ttfb, total, code) =
        streamed(&format!("{{ echo A; }} | \"{ping}\" -n 3 127.0.0.1\n"));
    assert_eq!(code, Some(0), "status");
    assert!(
        ttfb < LIVE_BOUND,
        "shell-writer pipeline withheld consumer output for {ttfb:?}"
    );
    assert!(
        total >= Duration::from_millis(1500),
        "ping -n 3 shape broken: finished in {total:?}"
    );
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("127.0.0.1"),
        "consumer output missing: {text:?}"
    );
}

#[test]
fn shell_writer_pipeline_output_passthrough() {
    let cat = resolve_tool("cat");
    let outcome = rubash_c(&format!("{{ echo A; echo B; }} | \"{cat}\"\n"));
    assert_eq!(outcome.code, Some(0), "status");
    assert_eq!(
        String::from_utf8_lossy(&outcome.stdout),
        "A\nB\n",
        "shell-writer payload must pass through unchanged"
    );
}

#[test]
fn shell_writer_pipeline_substitution_collects() {
    let cat = resolve_tool("cat");
    let outcome = rubash_c(&format!("x=$({{ echo A; }} | \"{cat}\"); echo \"[$x]\"\n"));
    assert_eq!(outcome.code, Some(0), "status");
    assert_eq!(
        String::from_utf8_lossy(&outcome.stdout),
        "[A]\n",
        "captured shell-writer pipeline must not stream past the substitution"
    );
}
