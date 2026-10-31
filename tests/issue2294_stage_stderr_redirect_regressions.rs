//! Issue unixwin/rubash#2294: on the shell-writer pipeline
//! composition, the producer stage's own `2>file` / `2>>file` was
//! ignored — the stage's stderr escaped to the shell's real
//! stderr and the file stayed empty.
//!
//! GNU runs every pipeline element's own redirections inside the
//! element (execute_cmd.c:2620 execute_pipeline; redir.c:237
//! do_redirections), so a stage's `2>file` claims its stderr
//! payload on every composition. rubash's stage spawner wires
//! such a stage's stderr as a capture pipe precisely so the
//! driver's routing walk (route_pipeline_stage_streams) places
//! the payload in the file — the sequential stage loop runs that
//! walk for every stage, but run_shell_writer_pipeline wrote the
//! producer's captured stderr straight to the ambient fd 2 under
//! a comment whose premise ("the element's body applies it
//! internally") holds only for compound producers. The driver
//! now routes the producer's captured streams with the loop's
//! exact gate: skip when the stage claims the pre-wired latch
//! (direct wiring, or a function stage that applied its
//! redirects to its fd table), skip when restricted-refused,
//! skip compound stages (internal application).
//!
//! Expectations byte-verified against GNU bash 5.3.20 (oracle
//! probes in the cluster workdir: p2294_stage0, p2294_mid and
//! the v_append / v_last2294 / v_three / v_control cells).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2294-{tag}-{}", std::process::id()));
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

/// Run `body` as a script inside `dir`.
fn run_in(dir: &Path, body: &str) -> (String, String, Option<i32>) {
    rubash_stdin(&format!("cd {} || exit 1\n{body}", dir.display()))
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
fn issue2294_stage0_producer_stderr_redirect_to_file() {
    // The issue reproducer: the external producer on the
    // shell-writer composition — the file must hold the payload
    // and nothing may escape to the shell's stderr.
    let dir = test_dir("stage0");
    let (stdout, stderr, code) = run_in(&dir, "sh -c 'echo e >&2' 2>e.txt | wc -c\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "0\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert!(
        wait_file_contents(&dir.join("e.txt"), "e\n"),
        "e.txt: {:?}",
        std::fs::read_to_string(dir.join("e.txt"))
    );
}

#[cfg(unix)]
#[test]
fn issue2294_stage0_producer_stderr_append() {
    // The append form: the routing walk applies `2>>` semantics,
    // so the pre-seeded content survives ahead of the payload.
    let dir = test_dir("append");
    std::fs::write(dir.join("e.txt"), "P\n").expect("seed e.txt");
    let (stdout, stderr, code) = run_in(&dir, "sh -c 'echo e >&2' 2>>e.txt | wc -c\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "0\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert!(
        wait_file_contents(&dir.join("e.txt"), "P\ne\n"),
        "e.txt: {:?}",
        std::fs::read_to_string(dir.join("e.txt"))
    );
}

#[cfg(unix)]
#[test]
fn issue2294_producer_stderr_redirect_three_stage() {
    // Same producer face with two downstream consumers on the
    // writer composition.
    let dir = test_dir("three");
    let (stdout, stderr, code) = run_in(&dir, "sh -c 'echo e >&2' 2>f.txt | cat | wc -c\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "0\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert!(
        wait_file_contents(&dir.join("f.txt"), "e\n"),
        "f.txt: {:?}",
        std::fs::read_to_string(dir.join("f.txt"))
    );
}

#[cfg(unix)]
#[test]
fn issue2294_mid_stage_stderr_redirect_to_file() {
    // Guard: a mid stage's `2>file` already routed correctly
    // through the sequential stage loop.
    let dir = test_dir("mid");
    let (stdout, stderr, code) = run_in(
        &dir,
        "printf 'x\\n' | sh -c 'cat; echo e >&2' 2>e2.txt | wc -c\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "2\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert!(
        wait_file_contents(&dir.join("e2.txt"), "e\n"),
        "e2.txt: {:?}",
        std::fs::read_to_string(dir.join("e2.txt"))
    );
}

#[cfg(unix)]
#[test]
fn issue2294_last_stage_stderr_redirect_to_file() {
    // Guard: the last stage's `2>file`, routed by the sequential
    // loop (a builtin producer declines the writer composition).
    let dir = test_dir("last");
    let (stdout, stderr, code) = run_in(&dir, "printf 'x\\n' | sh -c 'cat; echo e >&2' 2>f.txt\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "x\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert!(
        wait_file_contents(&dir.join("f.txt"), "e\n"),
        "f.txt: {:?}",
        std::fs::read_to_string(dir.join("f.txt"))
    );
}

#[cfg(unix)]
#[test]
fn issue2294_producer_stderr_without_redirect_still_escapes() {
    // Control: with no `2>` on the producer, the routing walk
    // must leave the captured stderr alone — it still belongs on
    // the shell's ambient fd 2.
    let dir = test_dir("control");
    let (stdout, stderr, code) = run_in(&dir, "sh -c 'echo e >&2' | wc -c\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "0\n", "stderr: {stderr}");
    assert_eq!(stderr, "e\n");
}
