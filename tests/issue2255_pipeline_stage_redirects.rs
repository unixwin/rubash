//! Issue unixwin/rubash#2255: a pipeline stage's OWN redirects
//! were never applied.
//!
//! GNU forks every pipeline element and runs ONE ordered
//! do_redirections walk in the child (redir.c:237 do_redirections;
//! execute_cmd.c:2620 execute_pipeline), so a stage's own numbered
//! bindings (`3<seed`), its fd-0 dup of such a binding (`<&3`), and
//! its numbered outputs (`3>file`) all take effect for the element
//! exactly as they do for a simple command. rubash's stage runners
//! composed a stage's fd 0 from the captured-input channel and ran
//! neither apply_external_redirects nor spawn_external_process's
//! fd delivery, so the stage's list was dead text: battery cells
//! e8/f6a/f6b of the #2235 probes printed 0 where GNU prints the
//! seed's byte count.
//!
//! This change routes the stage composition through the SAME
//! application path as a simple external command:
//! apply_external_stdin_redirect adjudicates fd 0 (now including
//! the dup/close winners), apply_external_fd_delivery — the segment
//! extracted from spawn_external_process — stamps/pins the ambient
//! table and opens the stage's own numbered redirects for the
//! child, and an inline-arm decline gate sends a non-builtin stage
//! carrying own numbered input bindings to that spawner (the text
//! arms hold no per-element fd table).
//!
//! Pin provenance: the f6a / numbered-output / rubash-child pins
//! exercise the delivery segment and are green with this change
//! alone. The e8/f6b and procsub pins additionally need #2235's
//! own-list dup replay + list materialization (PR #2253): they are
//! red on master, red with either change alone, and green once
//! both land — see the cluster REPORT for the integration note.
//! The two control pins are green on master and guard the routing.
//!
//! §8 extension (reopened #2255): in a >=3-stage pipeline whose
//! downstream stages are all concurrent-eligible externals, the
//! shell-writer composition (execute_shell_writer_pipeline)
//! claimed stage 0 and spawned the consumers on pre-created pipes
//! BEFORE running the producer, so a foreign anonymous pipe
//! occupied the producer child's fd 3 at pin time and the pin
//! guard skipped the stage's own binding — the child had no fd 3
//! at all (fd >= 4 targets survived; the 2-stage composition's
//! only foreign pipe is the stage's own stream pipe, which the
//! guard's stdio-alias check recognizes). The stage0 three/four-
//! stage pins below pin the admission decline that routes such
//! producers to the sequential stage loop; the middle-stage pin
//! is the control that already worked there.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2255-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
    std::fs::write(dir.join("seed.txt"), "seed-line\n").expect("write seed");
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

/// Run `body` as a script inside `dir` (seed.txt present there).
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

#[test]
fn issue2255_pipeline_stage_sh_child_inherits_own_numbered_input() {
    // Battery cell f6a: the stage's own `3<seed` binding must be
    // delivered to the foreign child at its fd number (the shared
    // spawn fd delivery), not dropped by the stage composition.
    let dir = test_dir("f6a");
    let (stdout, stderr, code) = run_in(&dir, "sh -c 'cat <&3' 3<seed.txt | wc -c\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "10\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
}

#[test]
fn issue2255_pipeline_stage_rubash_child_inherits_own_numbered_input() {
    // The env-channel face of the same delivery: a rubash child
    // restores the stage's fd-3 binding from the __RUBASH_FD_* stamps
    // (THIS_SH is rubash's own auto-detected executable path).
    let dir = test_dir("rubash-child");
    let (stdout, stderr, code) = run_in(&dir, "\"$THIS_SH\" -c 'cat <&3' 3<seed.txt | wc -c\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "10\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
}

#[test]
fn issue2255_pipeline_stage_own_dup_feeds_inline_command() {
    // Battery cells e8/f6b: the emulated cat stage declines to the
    // external spawner, whose fd-0 adjudication dups the stage's own
    // fd-3 binding (composes with #2235's own-list replay, PR #2253).
    let dir = test_dir("e8");
    let (stdout, stderr, code) = run_in(&dir, "cat 3<seed.txt <&3 | wc -c\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "10\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
}

#[test]
fn issue2255_pipeline_stage_procsub_input_dup() {
    // A stage-level list procsub binds fd 3 and the same-list dup
    // feeds the stage's stdin (composes with #2235's list-level
    // input materialization, PR #2253).
    let dir = test_dir("procsub");
    let (stdout, stderr, code) = run_in(&dir, "cat 3< <(printf 'a\\nb\\n') <&3 | wc -c\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "4\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
}

#[test]
fn issue2255_pipeline_stage_numbered_output_redirect() {
    // The write side of the delivery: the stage's own `3>o.txt`
    // opens for the child; nothing reaches the downstream stage.
    let dir = test_dir("output");
    let (stdout, stderr, code) = run_in(&dir, "sh -c 'echo o >&3' 3>o.txt | wc -c\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "0\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert!(
        wait_file_contents(&dir.join("o.txt"), "o\n"),
        "o.txt contents: {:?}",
        std::fs::read_to_string(dir.join("o.txt"))
    );
}

#[test]
fn issue2255_pipeline_stage_ambient_dup_control() {
    // Control (green on master): an AMBIENT fd-3 binding dup'd into
    // a stage's stdin already resolved through the fd table; the
    // decline gate must not disturb it.
    let dir = test_dir("ambient");
    let (stdout, stderr, code) = run_in(&dir, "exec 3<seed.txt\ncat <&3 | wc -c\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "10\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
}

#[test]
fn issue2255_pipeline_stage0_numbered_input_three_stage() {
    // §8 of the reopened issue: with two concurrent-eligible
    // consumers behind it, the shell-writer composition used to
    // claim stage 0 and lose its `3<seed` binding entirely (the
    // child had NO fd 3: dash reported `3: Bad file descriptor`).
    // GNU forks the element and applies its whole list in the
    // child, so the seed reaches wc through both consumers.
    let dir = test_dir("stage0-3stage");
    let (stdout, stderr, code) = run_in(&dir, "sh -c 'cat <&3' 3<seed.txt | cat | wc -c\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "10\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
}

#[test]
fn issue2255_pipeline_stage0_numbered_input_four_stage() {
    // Same §8 face, one more consumer in the chain.
    let dir = test_dir("stage0-4stage");
    let (stdout, stderr, code) = run_in(&dir, "sh -c 'cat <&3' 3<seed.txt | cat | cat | wc -c\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "10\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
}

#[test]
fn issue2255_pipeline_stage0_numbered_output_three_stage() {
    // §8 face, write side: the producer's own `3>o.txt` binding was
    // dropped the same way, so `>&3` wrote into the stage pipe
    // instead of the file (o.txt stayed empty, wc counted the
    // payload). GNU: the file gets the bytes, the pipe gets none.
    let dir = test_dir("stage0-output-3stage");
    let (stdout, stderr, code) = run_in(&dir, "sh -c 'echo o >&3' 3>o.txt | cat | wc -c\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "0\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert!(
        wait_file_contents(&dir.join("o.txt"), "o\n"),
        "o.txt contents: {:?}",
        std::fs::read_to_string(dir.join("o.txt"))
    );
}

#[test]
fn issue2255_pipeline_middle_stage_numbered_input_control() {
    // Control (green on master): a numbered binding on a MIDDLE
    // stage already declined every fast path (the concurrent specs
    // refuse list-only members) and was delivered by the sequential
    // stage loop; the §8 admission decline must not disturb it.
    let dir = test_dir("middle-stage");
    let (stdout, stderr, code) = run_in(
        &dir,
        "printf 'ab\\n' | sh -c 'cat <&3' 3<seed.txt | wc -c\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "10\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
}

#[test]
fn issue2255_pipeline_plain_control() {
    // Control (green on master): a redirect-free pipeline is
    // byte-identical before and after the routing change.
    let (stdout, stderr, code) = rubash_stdin("printf 'xy\\n' | cat | wc -c\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "3\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
}
