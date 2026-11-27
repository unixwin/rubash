//! Issue unixwin/rubash#2590: a spawned pipeline stage whose fd 0
//! arrives through a numbered dup chain (`3<&6 <&3`) advanced the
//! chain's source fd by the whole payload instead of the stage's
//! true consumption (the #2029 external-stage consumption seam,
//! cluster-H residual (2)).
//!
//! GNU semantics (bash 5.3 sources): execute_pipeline forks every
//! element with the shell's own descriptors (execute_cmd.c:2620) and
//! the element applies its redirections inside the fork
//! (execute_cmd.c:4617); a `<&N` dup shares the open file
//! description (redir.c:1193 dup2), so the element's reads advance
//! the shared offset by exactly what it read — `head -n 1` on a
//! regular file leaves the offset at the printed boundary (6 of
//! 12), a drainer leaves it at EOF, a non-reader leaves it at 0.
//!
//! The pre-fix rubash: the plain `head -n 1 <&6` shape folds
//! correctly through the inline stage arm, but the numbered chain
//! disqualifies that arm, head runs as a spawned stage, and the
//! own-input-binding resolution
//! (external_setup::endpoint_as_own_input_binding) drained the
//! source endpoint at bind time — the external-stage consumption
//! channel reported the payload length (12) rather than the
//! consumer's true read (6), so the trailing `cat <&6` read EOF.
//!
//! Expectations are the GNU Bash 5.3.20 outputs of the same scripts
//! (adjudication probe r2 plus same-seam variants, GNU oracle
//! deterministic). Pins run rubash as a subprocess and observe
//! stdout/stderr only.
//!
//! UNVERIFIED-BY-CAPTAIN: written code-only under the 2026-11-02
//! pace directive; the captain runs these stems centrally.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const DATA: &str = "LINE1\nLINE2\n";

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2590-{tag}-{}", std::process::id()));
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

/// Run `script` as rubash's stdin inside `dir` (data.txt pre-written).
fn run_script(dir: &Path, script: &str) -> (String, String, Option<i32>) {
    std::fs::write(dir.join("data.txt"), DATA).expect("write data.txt");
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

/// Probe r2 (the issue's headline shape): the spawned chain stage's
/// partial consumption folds by head's true read (6), so the
/// trailing dup read continues at LINE2.
#[test]
fn issue2590_spawned_chain_stage_folds_true_consumption() {
    let dir = test_dir("r2");
    let (out, err, code) = run_script(
        &dir,
        "exec 6<data.txt\nhead -n 1 3<&6 <&3 | wc -c\ncat <&6\n",
    );
    assert_eq!(out, "6\nLINE2\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

/// A partial byte-count consumer through the same chain: head -c 3
/// leaves the shared offset at 3, and the dup read serves the rest.
#[test]
fn issue2590_spawned_chain_stage_byte_partial() {
    let dir = test_dir("byte-partial");
    let (out, err, code) = run_script(
        &dir,
        "exec 6<data.txt\nhead -c 3 3<&6 <&3 | wc -c\ncat <&6\n",
    );
    assert_eq!(out, "3\nE1\nLINE2\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

/// A non-reader through the chain moves nothing: head -n 0 binds
/// the dup but reads zero bytes, so the dup read serves the whole
/// file afterwards.
#[test]
fn issue2590_spawned_chain_stage_non_reader_moves_nothing() {
    let dir = test_dir("non-reader");
    let (out, err, code) = run_script(
        &dir,
        "exec 6<data.txt\nhead -n 0 3<&6 <&3 | wc -c\ncat <&6\n",
    );
    assert_eq!(out, "0\nLINE1\nLINE2\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

/// Negative control: a draining consumer through the chain still
/// folds to EOF — the fold must not under-advance a full read.
#[test]
fn issue2590_spawned_chain_stage_drainer_folds_to_eof() {
    let dir = test_dir("drainer");
    let (out, err, code) = run_script(
        &dir,
        "exec 6<data.txt\ncat 3<&6 <&3 | wc -c\ncat <&6 | wc -c\n",
    );
    assert_eq!(out, "12\n0\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

/// The same bind seam without a pipeline (the plain external path
/// shares endpoint_as_own_input_binding): a partial reader through
/// the numbered chain leaves the shared offset at the boundary.
#[test]
fn issue2590_plain_chain_command_folds_true_consumption() {
    let dir = test_dir("plain");
    let (out, err, code) = run_script(&dir, "exec 6<data.txt\nhead -n 1 3<&6 <&3\ncat <&6\n");
    assert_eq!(out, "LINE1\nLINE2\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}
