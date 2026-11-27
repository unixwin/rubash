//! Issue unixwin/rubash#2502: pipeline stage-0 payload delivery for
//! fd-alias stdin winners — the surviving residuals of the #2476/#2479
//! wave (cluster B harvest).
//!
//! GNU semantics (bash 5.3 sources): execute_pipeline forks every
//! element with the shell's own descriptors (execute_cmd.c:2620) and
//! the element applies its own redirections inside the fork
//! (execute_cmd.c:4617), so there is no payload copy anywhere: a
//! `<&N` dup (redir.c:1193 dup2) makes the element read the SHARED
//! open file description its chain ends at, and a `< /dev/fd/N`
//! alias-spelling open is a plain open() of the magic link
//! (redir.c:725) that re-opens a regular-file descriptor's target
//! fresh without moving the descriptor. Rubash resolves the payload
//! by draining a descriptor and materializes a buffered copy for the
//! stage's descriptor-channel consumers; the copy must land on the
//! descriptor the stage actually reads through.
//!
//! The pre-fix rubash materialized onto the fd the WINNER'S SPELLING
//! names: for `cat 3<&6 <&3` the resolution drained fd 6 (the dup
//! chain's terminal) but the copy landed on fd 3, which no consumer
//! reads — the stage read EOF and printed 0. For `cat < /dev/fd/6`
//! the resolution served a fresh open, yet the materialization still
//! replaced fd 6's file endpoint with the payload copy and the fold
//! advanced it to EOF, so the same command a second time in the same
//! process printed 0 instead of a fresh 12.
//!
//! Expectations are the GNU Bash 5.3.20 outputs of the same scripts
//! (oracle binary run side by side in the cluster workdir). Pins run
//! rubash as a subprocess and observe stdout/stderr only.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SEED: &str = "hello world\n";
const SEED2: &str = "LINE1\nLINE2\n";

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2502-{tag}-{}", std::process::id()));
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

/// Run `script` as rubash's stdin inside `dir` (seeds pre-written).
fn run_script(dir: &Path, script: &str) -> (String, String, Option<i32>) {
    std::fs::write(dir.join("seed"), SEED).expect("write seed");
    std::fs::write(dir.join("seed2"), SEED2).expect("write seed2");
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

/// The issue's headline shape A: a true dup feeds the pipeline's
/// stdin. The payload lives on the dup chain's terminal (fd 6), and
/// the stage's consumption is the shared description's consumption —
/// a later direct dup read sees EOF, exactly like GNU.
#[test]
fn issue2502_dup_fed_pipeline_delivers_and_drains_terminal() {
    let dir = test_dir("dup-fed");
    let (out, err, code) = run_script(&dir, "exec 6<seed\ncat 3<&6 <&3 | wc -c\ncat <&6 | wc -c\n");
    assert_eq!(out, "12\n0\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

/// The issue's headline shape B: the alias-spelling winner re-opens
/// fresh on EVERY run in the same process — the descriptor is never
/// drained and never replaced by the payload copy.
#[test]
fn issue2502_spelling_fed_pipeline_reopens_fresh_every_run() {
    let dir = test_dir("spelling-fed");
    let (out, err, code) = run_script(
        &dir,
        "exec 6<seed\ncat < /dev/fd/6 | wc -c\ncat < /dev/fd/6 | wc -c\n",
    );
    assert_eq!(out, "12\n12\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

/// The full probe sequence from the issue, in one process: dup-fed,
/// spelling-fed twice, alias-open-fed, then a rebound direct dup —
/// every shape prints the seed size except the final drained dup.
#[test]
fn issue2502_full_probe_sequence_matches_gnu() {
    let dir = test_dir("sequence");
    let (out, err, code) = run_script(
        &dir,
        "exec 6<seed\ncat 3<&6 <&3 | wc -c\ncat < /dev/fd/6 | wc -c\ncat < /dev/fd/6 | wc -c\ncat 3< /dev/fd/6 <&3 | wc -c\nexec 6<seed\ncat <&6 | wc -c\ncat <&6 | wc -c\n",
    );
    assert_eq!(out, "12\n12\n12\n12\n12\n0\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

/// After a dup-fed pipeline drained fd 6 through the materialized
/// copy, an alias-spelling open of the same slot still opens the
/// underlying file fresh (the copy remembers its origin).
#[test]
fn issue2502_spelling_open_after_dup_drain_reopens_origin() {
    let dir = test_dir("origin");
    let (out, err, code) = run_script(
        &dir,
        "exec 6<seed\ncat 3<&6 <&3 | wc -c\ncat < /dev/fd/6 | wc -c\n",
    );
    assert_eq!(out, "12\n12\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

/// A sibling dup of the drained description (`exec 7<&6` shares the
/// open file description) sees the stage's consumption through the
/// shared copy: after the dup-fed pipeline, fd 7 is at EOF too.
#[test]
fn issue2502_sibling_dup_shares_stage_consumption() {
    let dir = test_dir("sibling");
    let (out, err, code) = run_script(
        &dir,
        "exec 6<seed\nexec 7<&6\ncat 3<&6 <&3 | wc -c\ncat <&7 | wc -c\n",
    );
    assert_eq!(out, "12\n0\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

/// Partial consumption through a direct dup winner keeps GNU's
/// seek-back fold on the delivered descriptor: head prints the
/// first line, the next reader of the same descriptor continues
/// where the shared offset stopped.
#[test]
fn issue2502_partial_consumption_folds_back_on_terminal() {
    let dir = test_dir("partial");
    let (out, err, code) = run_script(&dir, "exec 6<seed2\nhead -n 1 <&6 | wc -c\ncat <&6\n");
    assert_eq!(out, "6\nLINE2\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}
