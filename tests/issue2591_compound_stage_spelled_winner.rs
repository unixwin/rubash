//! Issue unixwin/rubash#2591: compound-group and subshell pipeline
//! stages bound a spelled `/dev/fd/N` stdin winner by dup'ing the
//! donor endpoint (shared offset) instead of opening it fresh
//! (cluster-H residual (5), narrowed by isolation probes).
//!
//! GNU semantics (bash 5.3 sources): a pipeline element applies its
//! redirections in the forked child over the shell's descriptors
//! (execute_cmd.c:2620 pipeline fork; element redirection
//! application in the child). A `<&N` dup shares the open file
//! description (redir.c:1193 dup2 — reads advance the shared
//! offset), while a spelled `/dev/fd/N` target on a HAVE_DEV_FD
//! build is a plain open of the magic symlink (redir.c:725
//! redir_open; the RF_DEVFD dup emulation at redir.c:601-603
//! compiles only without HAVE_DEV_FD) — a fresh description at
//! offset 0, the donor's offset unmoved.
//!
//! The pre-fix rubash: the compound-stage funnel
//! (execute_compound_pipeline_stage) bound the stage clone's fd 0
//! with dup_input for BOTH winner kinds, so after `head -n 1 <&6`
//! left fd 6 at offset 6, `{ cat; } < /dev/fd/6 | wc -c` served the
//! 6-byte remainder instead of the fresh 12. Function stages and
//! redirects spelled INSIDE a group/function body already took a
//! correct path and are pinned here as negative controls.
//!
//! Expectations are the GNU Bash 5.3.20 outputs of the same scripts
//! (adjudication probes r5/r5b/r5c/r5d/r5e, GNU oracle
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
    let dir = std::env::temp_dir().join(format!("rubash-issue2591-{tag}-{}", std::process::id()));
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

/// Probe r5c: a brace-group stage with a spelled winner fresh-opens
/// (12), it does not continue at the donor's offset (6).
#[test]
fn issue2591_brace_group_stage_spelled_winner_opens_fresh() {
    let dir = test_dir("r5c");
    let (out, err, code) = run_script(
        &dir,
        "exec 6<data.txt\nhead -n 1 <&6\n{ cat; } < /dev/fd/6 | wc -c\n",
    );
    assert_eq!(out, "LINE1\n12\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

/// Probe r5d: a subshell stage with a spelled winner fresh-opens
/// the same way.
#[test]
fn issue2591_subshell_stage_spelled_winner_opens_fresh() {
    let dir = test_dir("r5d");
    let (out, err, code) = run_script(
        &dir,
        "exec 6<data.txt\nhead -n 1 <&6\n( cat ) < /dev/fd/6 | wc -c\n",
    );
    assert_eq!(out, "LINE1\n12\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

/// Probe r5 (full sequence): the compound spelled shapes read 12,
/// the compound DUP shape in between still shares the description
/// (6, draining it), and the function/plain spelled shapes stay 12.
#[test]
fn issue2591_full_r5_sequence_matches_gnu() {
    let dir = test_dir("r5");
    let (out, err, code) = run_script(
        &dir,
        "exec 6<data.txt\nhead -n 1 <&6\n{ cat; } < /dev/fd/6 | wc -c\n{ cat; } <&6 | wc -c\ng() { cat; }\ng < /dev/fd/6 | wc -c\ncat < /dev/fd/6 | wc -c\n",
    );
    assert_eq!(out, "LINE1\n12\n6\n12\n12\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

/// Negative control: a brace-group stage with a DUP winner keeps
/// sharing the description — it serves the remainder and drains
/// the shared offset for the next reader.
#[test]
fn issue2591_brace_group_stage_dup_winner_still_shares() {
    let dir = test_dir("dup-winner");
    let (out, err, code) = run_script(
        &dir,
        "exec 6<data.txt\nhead -n 1 <&6\n{ cat; } <&6 | wc -c\ncat <&6 | wc -c\n",
    );
    assert_eq!(out, "LINE1\n6\n0\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

/// Probe r5b (negative control): a spelled redirect INSIDE a
/// function body, the function itself a pipeline stage, was
/// already fresh before the fix and must stay fresh; the donor
/// offset is unmoved (the trailing dup read sees LINE2).
#[test]
fn issue2591_function_internal_spelled_stays_fresh() {
    let dir = test_dir("r5b");
    let (out, err, code) = run_script(
        &dir,
        "exec 6<data.txt\nhead -n 1 <&6\nf() { cat < /dev/fd/6; }\nf | wc -c\ncat <&6\n",
    );
    assert_eq!(out, "LINE1\n12\nLINE2\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

/// Probe r5e (negative control): a spelled redirect inside a brace
/// group (not on the group itself) was already fresh before the
/// fix and must stay fresh, donor offset unmoved.
#[test]
fn issue2591_group_internal_spelled_stays_fresh() {
    let dir = test_dir("r5e");
    let (out, err, code) = run_script(
        &dir,
        "exec 6<data.txt\nhead -n 1 <&6\n{ cat < /dev/fd/6; } | wc -c\ncat <&6\n",
    );
    assert_eq!(out, "LINE1\n12\nLINE2\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}
