//! rubash#3985 — a `/dev/full` target was refused at the shell's
//! own binding layer (or silently dropped / leaked on delivery)
//! instead of being bound so the child's write could fail ENOSPC.
//!
//! GNU redir_open (redir.c:725) consults only its special-filename
//! list (/dev/fd, /dev/std*, /dev/tcp|udp) and opens anything else —
//! /dev/full included — with a plain open(2); the forked or exec'd
//! image inherits that device, its write fails ENOSPC inside the
//! child, and the child diagnoses itself. rubash's create / append /
//! readwrite-probe opens of the device substituted a temp-file sink
//! (the #1898 stand-in), so on any host whose temp dir rejects the
//! create the redirect died at binding time with the sink's raw
//! EACCES ("Permission denied", no child ever ran); and the exec
//! spawn walk resolved a fd-table FullDevice binding as Ambient —
//! the process's own descriptor at that number — so the exec'd
//! image wrote to an unrelated descriptor and exited 0.
//!
//! Expectations byte-verified against GNU bash 5.3.20 with
//! script-file probes (oracle and rubash driving the same host
//! /bin/echo produce byte-identical output). The child's own
//! write-error phrasing belongs to the host's coreutils — GNU
//! coreutils says "/bin/echo: write error: No space left on
//! device" — so the pins assert the invariant shape: the ENOSPC
//! report is present, the shell's binding refusal is absent, and
//! stdout / status are exact (the same environment caveat as the
//! #3312 cells in issue3311_exec_child_fd_delivery).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

/// The failure must be the child's own ENOSPC write report,
/// never the shell refusing the binding at redirect time.
fn assert_child_enospc(stderr: &str) {
    assert!(
        stderr.contains("No space left on device"),
        "child must diagnose its own ENOSPC write, stderr: {stderr}"
    );
    assert!(
        !stderr.contains("Permission denied"),
        "the shell must not refuse the binding, stderr: {stderr}"
    );
    assert!(
        !stderr.contains("No such file or directory"),
        "the shell must not refuse the binding, stderr: {stderr}"
    );
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue3985-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Run `body` as a script FILE under rubash with cwd = dir.
/// Returns (stdout, stderr, status).
fn rubash_script(dir: &Path, body: &str) -> (String, String, Option<i32>) {
    rubash_script_env(dir, body, false)
}

/// With `hostile_tmp`, the child rubash runs with TMPDIR pointing
/// into a nonexistent directory: any temp-file stand-in for the
/// device open fails there, which is exactly the host condition
/// under which the binding-layer refusal appeared. GNU has no
/// temp file in this path at all.
fn rubash_script_env(dir: &Path, body: &str, hostile_tmp: bool) -> (String, String, Option<i32>) {
    let script = dir.join("case.sh");
    fs::write(&script, body).expect("write script fixture");
    let mut cmd = Command::new(RUBASH);
    cmd.arg(&script).current_dir(dir).stdin(Stdio::null());
    if hostile_tmp {
        cmd.env("TMPDIR", dir.join("missing").join("sub"));
    }
    let output = cmd.output().expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[cfg(unix)]
#[test]
fn issue3985_subshell_exec_dup_onto_full_fails_in_child() {
    let dir = scratch("dup3");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        "( exec /bin/echo hi >&3 ) 3>/dev/full; echo \"outer rc=$?\"\n",
    );
    assert_eq!(stdout, "outer rc=1\n");
    assert_child_enospc(&stderr);
    assert!(
        !stderr.contains("hi"),
        "the payload must not leak to the shell's stderr, stderr: {stderr}"
    );
}

#[cfg(unix)]
#[test]
fn issue3985_subshell_exec_dup_onto_full_hostile_tmpdir() {
    let dir = scratch("dup3-tmp");
    let (stdout, stderr, _) = rubash_script_env(
        &dir,
        "( exec /bin/echo hi >&3 ) 3>/dev/full; echo \"outer rc=$?\"\n",
        true,
    );
    assert_eq!(stdout, "outer rc=1\n");
    assert_child_enospc(&stderr);
    assert!(
        !stderr.contains("hi"),
        "the payload must not leak to the shell's stderr, stderr: {stderr}"
    );
}

#[cfg(unix)]
#[test]
fn issue3985_top_level_exec_full_hostile_tmpdir() {
    let dir = scratch("top-tmp");
    let (stdout, stderr, status) = rubash_script_env(&dir, "exec /bin/echo hi >/dev/full\n", true);
    assert_eq!(stdout, "");
    assert_child_enospc(&stderr);
    assert_eq!(status, Some(1));
}

#[cfg(unix)]
#[test]
fn issue3985_append_exec_full_hostile_tmpdir() {
    let dir = scratch("append-tmp");
    let (stdout, stderr, _) = rubash_script_env(
        &dir,
        "( exec /bin/echo hi >>/dev/full ); echo \"append rc=$?\"\n",
        true,
    );
    assert_eq!(stdout, "append rc=1\n");
    assert_child_enospc(&stderr);
}

#[cfg(unix)]
#[test]
fn issue3985_readwrite_full_hostile_tmpdir() {
    let dir = scratch("rw-tmp");
    let (stdout, stderr, _) = rubash_script_env(
        &dir,
        "( exec /bin/echo hi >&9 ) 9<>/dev/full; echo \"rw rc=$?\"\n",
        true,
    );
    assert_eq!(stdout, "rw rc=1\n");
    assert_child_enospc(&stderr);
}

/// A FullDevice binding already standing in the shell's own table
/// (a permanent `exec 1>/dev/full`) reaches the next exec'd image
/// as the device itself — the payload must not leak to the real
/// stdout and the image must fail on its own write.
#[cfg(unix)]
#[test]
fn issue3985_ambient_std_binding_reaches_exec_image() {
    let dir = scratch("ambient1");
    let (stdout, stderr, status) = rubash_script(&dir, "exec 1>/dev/full\nexec /bin/echo hi\n");
    assert_eq!(stdout, "");
    assert_child_enospc(&stderr);
    assert_eq!(status, Some(1));
}

/// Control: the same dup shape onto a regular file lands and
/// succeeds, before and after the fix.
#[cfg(unix)]
#[test]
fn issue3985_subshell_exec_dup_onto_file_still_lands() {
    let dir = scratch("control");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        "( exec /bin/echo hi >&3 ) 3>reg.txt; echo \"rc=$? f=[$(cat reg.txt)]\"\n",
    );
    assert_eq!(stdout, "rc=0 f=[hi]\n");
    assert_eq!(stderr, "");
}
