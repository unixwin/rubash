//! Issue unixwin/rubash#2589: a `/dev/fd/N` OPERAND served to a
//! spawned consumer after a pipeline stage consumed the donor fd.
//!
//! GNU semantics (bash 5.3 sources): a spelled `/dev/fd/N` on a
//! HAVE_DEV_FD build is a plain open of the magic symlink — a fresh
//! open file description at offset 0 that never shares or moves the
//! donor fd's offset. The RF_DEVFD dup emulation
//! (redir.c:601-603, `_redir_special_filenames`) compiles only
//! `#if !defined (HAVE_DEV_FD)`; with HAVE_DEV_FD the spelling falls
//! through `redir_open` (redir.c:725-) to open(). A `<&N` dup, by
//! contrast, shares the open file description (redir.c:1141-).
//!
//! The pre-fix rubash: a pipeline stage that consumed fd N left the
//! fd-table slot holding the stage's materialized copy (a Text
//! endpoint remembering its reopen origin, #2502), and the spawned
//! operand channel (dev_fd_operands) served the copy's CURRENT bytes
//! — 0 after a draining stage, the remainder after a partial one —
//! instead of fresh-serving the origin the way the redirect path
//! (fd_alias_reopen_path) already did.
//!
//! Expectations are the GNU Bash 5.3.20 outputs of the same scripts
//! (adjudication probes r1a–r1d in the cluster-H residuals report,
//! run side by side: GNU 2 runs, rubash 3 runs, all deterministic,
//! rc=0, stderr empty on both shells). Pins run rubash as a
//! subprocess and observe stdout/stderr only.
//!
//! UNVERIFIED-BY-CAPTAIN: written code-only under the 2026-11-02
//! pace directive; the captain runs these stems centrally.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const DATA: &str = "LINE1\nLINE2\n";

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2589-{tag}-{}", std::process::id()));
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

/// Probe r1a: a dup-fed stage drains fd 6; the bare spawned operand
/// afterwards must still fresh-open the origin (12), not serve the
/// drained copy (0).
#[test]
fn issue2589_operand_after_draining_stage_reopens_origin() {
    let dir = test_dir("r1a");
    let (out, err, code) = run_script(
        &dir,
        "exec 6<data.txt\ncat 3<&6 <&3 | wc -c\nwc -c /dev/fd/6\n",
    );
    assert_eq!(out, "12\n12 /dev/fd/6\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

/// Probe r1b: a partial stage leaves fd 6 at offset 6; the operand
/// in a spawned pipeline stage fresh-opens (12, not the copy
/// remainder 6), and the operand open does not move fd 6 — the
/// trailing dup read still sees LINE2.
#[test]
fn issue2589_operand_after_partial_stage_reopens_origin() {
    let dir = test_dir("r1b");
    let (out, err, code) = run_script(
        &dir,
        "exec 6<data.txt\nhead -n 1 <&6 | wc -c\ncat /dev/fd/6 | wc -c\ncat <&6\n",
    );
    assert_eq!(out, "6\n12\nLINE2\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

/// Probe r1c (negative control): after the same draining stage, the
/// in-shell emulated `cat` operand already fresh-served correctly
/// before the fix and must stay correct.
#[test]
fn issue2589_emulated_operand_after_stage_stays_fresh() {
    let dir = test_dir("r1c");
    let (out, err, code) = run_script(
        &dir,
        "exec 6<data.txt\ncat 3<&6 <&3 | wc -c\ncat /dev/fd/6\n",
    );
    assert_eq!(out, "12\nLINE1\nLINE2\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

/// Probe r1d (negative control): purely in-shell consumption leaves
/// the origin File endpoint in the slot; the spawned operand was
/// already correct before the fix and must stay correct.
#[test]
fn issue2589_operand_after_inshell_consumption_stays_fresh() {
    let dir = test_dir("r1d");
    let (out, err, code) = run_script(
        &dir,
        "exec 6<data.txt\nwc -c /dev/fd/6\nIFS= read -r line <&6\nwc -c /dev/fd/6\n",
    );
    assert_eq!(out, "12 /dev/fd/6\n12 /dev/fd/6\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}
