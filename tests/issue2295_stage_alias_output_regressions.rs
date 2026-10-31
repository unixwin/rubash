//! Issue unixwin/rubash#2295: the pipeline-stage face of the
//! /dev/fd alias-output family. A stage's numbered output redirect
//! through an alias PATH spelling (`3>/dev/fd/6`) was skipped by
//! the spawn delivery walk (its generic alias arm only skipped),
//! so the child got no descriptor at its slot (EBADF) — while the
//! parent-side routing probe had already fresh-opened (truncated)
//! the target, destroying its contents without delivering
//! anything.
//!
//! GNU: the spelling is an open over the alias layer (redir.c:725
//! redir_open via do_redirections, redir.c:237; the RF_DEVFD dup
//! special case at redir.c:602-629 compiles only without
//! HAVE_DEV_FD), so the stage's child receives a FRESH open of the
//! resolved regular file and its bytes land in the target. The
//! same delivery walk serves simple (non-pipeline) commands, so
//! the simple-command numbered shapes are pinned here too.
//! Expectations are the GNU Bash 5.3.20 outputs of
//! probes/oracle-pins.sh (cluster workdir). Pins run rubash as a
//! subprocess and observe stdout/stderr/files only.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const TEN: &str = "0123456789";

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2295-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
    dir
}

fn collect(child: std::process::Child) -> (String, String, Option<i32>) {
    let mut child = child;
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

/// Run `script` as rubash's stdin inside `dir` (seed.txt pre-written
/// with the ten-byte contents the issue's reproducer uses).
fn run_script(dir: &Path, script: &str) -> (String, String, Option<i32>) {
    std::fs::write(dir.join("seed.txt"), TEN).expect("write seed.txt");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(script.as_bytes())
        .expect("write script");
    drop(child.stdin.take());
    collect(child)
}

fn read_seed(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("seed.txt")).expect("read back seed.txt")
}

/// The issue's reproducer: nothing reaches the pipe (wc counts 0),
/// and the target ends up holding the stage's byte — crucially it
/// is NOT left truncated-empty (the pre-fix shape: parent probe
/// truncated, child EBADF'd).
#[test]
fn issue2295_stage_alias_output_delivers_into_target() {
    let dir = test_dir("stage");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 6<seed.txt\n/bin/sh -c 'echo o >&3' 3>/dev/fd/6 | wc -c\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "0\n", "stderr: {stderr}");
    assert_eq!(read_seed(&dir), "o\n");
}

/// Stage append form: prior contents preserved, stage bytes
/// appended.
#[test]
fn issue2295_stage_alias_append_preserves_target() {
    let dir = test_dir("stage-append");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 6<seed.txt\n/bin/sh -c 'echo o >&3' 3>>/dev/fd/6 | wc -c\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "0\n", "stderr: {stderr}");
    assert_eq!(read_seed(&dir), format!("{TEN}o\n"));
}

/// Mid-pipeline stage (not the first): same delivery.
#[test]
fn issue2295_mid_stage_alias_output_delivers_into_target() {
    let dir = test_dir("mid-stage");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 6<seed.txt\n/bin/true | /bin/sh -c 'echo m >&3' 3>/dev/fd/6 | wc -c\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "0\n", "stderr: {stderr}");
    assert_eq!(read_seed(&dir), "m\n");
}

/// Simple-command (non-pipeline) numbered shape through the same
/// delivery walk: the child writes into the target.
#[test]
fn issue2295_simple_numbered_alias_output_writes_target() {
    let dir = test_dir("simple");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 6<seed.txt\n/bin/sh -c 'echo o >&3' 3>/dev/fd/6\ncat seed.txt\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "o\n", "stderr: {stderr}");
    assert_eq!(read_seed(&dir), "o\n");
}

/// Simple-command numbered append form.
#[test]
fn issue2295_simple_numbered_alias_append_preserves_target() {
    let dir = test_dir("simple-append");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 6<seed.txt\n/bin/sh -c 'echo o >&3' 3>>/dev/fd/6\ncat seed.txt\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, format!("{TEN}o\n"), "stderr: {stderr}");
    assert_eq!(read_seed(&dir), format!("{TEN}o\n"));
}

/// Control: the input direction through the stage face already
/// worked and must keep working — the stage reads the target's
/// contents through the alias and the target is untouched.
#[test]
fn issue2295_input_direction_control_unchanged() {
    let dir = test_dir("input-control");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 6<seed.txt\n/bin/sh -c 'cat <&3' 3</dev/fd/6 | wc -c\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "10\n", "stderr: {stderr}");
    assert_eq!(read_seed(&dir), TEN);
}
