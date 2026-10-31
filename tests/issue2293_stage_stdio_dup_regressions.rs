//! Issue unixwin/rubash#2293: a pipeline stage's numbered dup of
//! a stdio slot (`3>&1`, `3>&2`, `3<&0`) must copy the descriptor
//! as bound AT THE DUP'S POSITION in the element's own list.
//!
//! GNU binds every pipeline element's fds 0/1/2 to the
//! inter-element pipes BEFORE the element's own do_redirections
//! walk runs (execute_cmd.c:2620 execute_pipeline; redir.c:237
//! do_redirections, the dup2 itself at redir.c:1193), so the
//! at-position binding of a stdio slot on the stage face is the
//! STAGE CHANNEL — the payload feeding fd 0, the capture pipes on
//! fds 1/2 — never the shell's ambient descriptor and never the
//! binding a later list entry installs on the slot.
//!
//! Two faces of that rule:
//! - The plain dup shapes (mid/last stage, `3>&1`/`3>&2`, the
//!   combined producer) were already delivered by #2278's shared
//!   delivery once the spawner wired the stage channels; they are
//!   guards here.
//! - The REBOUND shapes are this issue's fix: when a later entry
//!   rebinds the dup's source slot (`3>&1 1>f.txt`,
//!   `3<&0 <seed2`), the stage spawner's delivery captured the
//!   shell's AMBIENT descriptor parent-side (the simple-command
//!   arm of deliver_numbered_dup), so the dup read/wrote the
//!   shell's own stdio instead of the stage channel. The delivery
//!   now takes the spawner's StageStdioContext: fds 1/2 keep the
//!   child-local pin (the wired slot IS the channel), and fd 0
//!   pins the payload carrier the spawner materializes when the
//!   stdin face wires the later winner into the child's fd 0.
//!
//! Expectations byte-verified against GNU bash 5.3.20 (oracle
//! probes in the cluster workdir, probes/q3, probes/r2 and the
//! v_last_* cells).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2293-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
    std::fs::write(dir.join("seed.txt"), "seed-line\n").expect("write seed");
    std::fs::write(dir.join("seed2.txt"), "SEED2-LINE\n").expect("write seed2");
    dir
}

fn rubash_stdin(script: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(RUBASH)
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

/// Run `body` as a script inside `dir` (seed.txt/seed2.txt there).
fn run_in(dir: &Path, body: &str) -> (String, String, Option<i32>) {
    rubash_stdin(&format!("cd {} || exit 1\n{body}", dir.display()))
}

/// Run `script` with the shell's own stdin bound to `stdin_file`:
/// the ambient-stdin witness for the fd-0 rebound pin (the buggy
/// delivery read the ambient file instead of the stage payload).
fn rubash_in_stdin_file(
    dir: &Path,
    script: &str,
    stdin_file: &Path,
) -> (String, String, Option<i32>) {
    let file = std::fs::File::open(stdin_file).expect("open stdin file");
    let output = Command::new(RUBASH)
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .stdin(Stdio::from(file))
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Poll until `path` has exactly `expected` contents or the deadline.
fn wait_file_contents(path: &Path, expected: &str) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if std::fs::read_to_string(path).is_ok_and(|text| text == expected) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    std::fs::read_to_string(path).is_ok_and(|text| text == expected)
}

#[cfg(unix)]
#[test]
fn issue2293_mid_stage_dup_of_stdout_channel() {
    // Guard (delivered by #2278's shared delivery): the mid
    // stage's `3>&1` copies the capture pipe bound to fd 1.
    let dir = test_dir("mid-dup1");
    let (stdout, stderr, code) = run_in(&dir, "echo hi | sh -c 'echo o >&3' 3>&1 | wc -c\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "2\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
}

#[cfg(unix)]
#[test]
fn issue2293_last_stage_dup_of_stdout_channel() {
    // Guard: the last stage's `3>&1` lands on the pipeline
    // stdout alongside the stage's own fd 1 payload.
    let dir = test_dir("last-dup1");
    let (stdout, stderr, code) = run_in(&dir, "printf 'x\\n' | sh -c 'cat; echo o >&3' 3>&1\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "x\no\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
}

#[cfg(unix)]
#[test]
fn issue2293_mid_stage_dup_of_stderr_channel() {
    // Guard: `3>&2` mid-pipeline copies the stage's stderr
    // channel; the payload reaches the shell's stderr, and only
    // the stage's fd 1 stream feeds the count.
    let dir = test_dir("mid-dup2");
    let (stdout, stderr, code) = run_in(
        &dir,
        "printf 'x\\n' | sh -c 'cat; echo o >&3' 3>&2 | wc -c\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "2\n", "stderr: {stderr}");
    assert_eq!(stderr, "o\n");
}

#[cfg(unix)]
#[test]
fn issue2293_last_stage_dup_of_stderr_channel() {
    // Guard: `3>&2` on the last stage routes the dup payload to
    // the shell's stderr while fd 1 keeps the pipe payload.
    let dir = test_dir("last-dup2");
    let (stdout, stderr, code) = run_in(&dir, "printf 'x\\n' | sh -c 'cat; echo o >&3' 3>&2\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "x\n", "stderr: {stderr}");
    assert_eq!(stderr, "o\n");
}

#[cfg(unix)]
#[test]
fn issue2293_combined_producer_file_and_stdio_dup() {
    // Guard (the issue's combined-producer consequence shape):
    // a stage-0 producer carrying BOTH a numbered file binding
    // and a pure stdio dup — the file binding feeds `cat <&3`
    // (10 bytes) and the dup feeds the pipeline (2 bytes).
    let dir = test_dir("combined");
    let (stdout, stderr, code) = run_in(
        &dir,
        "sh -c 'cat <&3; echo o >&4' 3<seed.txt 4>&1 | wc -c\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "12\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
}

#[cfg(unix)]
#[test]
fn issue2293_mid_stage_rebound_stdout_dup_reads_stage_channel() {
    // The fix: `3>&1 1>f.txt` — the dup precedes the entry that
    // moves fd 1 to the file, so fd 3 keeps the STAGE PIPE (the
    // at-position binding), not the shell's ambient stdout.
    // GNU: the count sees `o` (2 bytes), f.txt holds only `s`,
    // and nothing escapes to the shell's real stdout.
    let dir = test_dir("mid-rebound1");
    let (stdout, stderr, code) = run_in(
        &dir,
        "printf 'x\\n' | sh -c 'echo o >&3; echo s' 3>&1 1>f.txt | wc -c\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "2\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert!(
        wait_file_contents(&dir.join("f.txt"), "s\n"),
        "f.txt: {:?}",
        std::fs::read_to_string(dir.join("f.txt"))
    );
}

#[cfg(unix)]
#[test]
fn issue2293_last_stage_rebound_stdout_dup_reads_stage_channel() {
    // The fix, last-stage face: fd 3 keeps the pipeline stdout
    // while fd 1 (and its payload) moves to f.txt.
    let dir = test_dir("last-rebound1");
    let (stdout, stderr, code) = run_in(
        &dir,
        "printf 'x\\n' | sh -c 'cat; echo o >&3; echo s' 3>&1 1>f.txt\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "o\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert!(
        wait_file_contents(&dir.join("f.txt"), "x\ns\n"),
        "f.txt: {:?}",
        std::fs::read_to_string(dir.join("f.txt"))
    );
}

#[cfg(unix)]
#[test]
fn issue2293_mid_stage_dup_of_stdin_channel_no_rebind() {
    // Guard: `3<&0` with no later fd-0 entry copies the stage's
    // payload pipe; both cats read the one payload in order.
    let dir = test_dir("mid-dup0");
    let (stdout, stderr, code) = run_in(
        &dir,
        "printf 'PAY\\n' | sh -c 'cat <&3; cat' 3<&0 | wc -c\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "4\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
}

#[cfg(unix)]
#[test]
fn issue2293_mid_stage_rebound_stdin_dup_reads_payload_not_ambient() {
    // The fix: `3<&0 <seed2.txt` — the dup precedes the fd-0
    // winner, so fd 3 keeps the STAGE PAYLOAD (`PAY\n`, 4 bytes)
    // while fd 0 takes seed2 (11 bytes): 15 total. The buggy
    // delivery captured the shell's ambient stdin (seed.txt,
    // 10 bytes) and printed 21. The shell's stdin is bound to
    // seed.txt so the ambient capture is observable.
    let dir = test_dir("mid-rebound0");
    let amb = dir.join("seed.txt");
    let (stdout, stderr, code) = rubash_in_stdin_file(
        &dir,
        "printf 'PAY\\n' | sh -c 'cat <&3; cat' 3<&0 <seed2.txt | wc -c",
        &amb,
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "15\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
}
