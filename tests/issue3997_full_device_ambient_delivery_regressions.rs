//! rubash#3997 — an fd-table `/dev/full` binding never reached an
//! external child: the ambient fd>=3 delivery (the `__RUBASH_FD_*`
//! stamp channel for rubash children, the pre_exec pin channel for
//! foreign children) carries only handle-backed endpoints, and the
//! FullDevice endpoint is symbolic (rubash#1898), so the slot was
//! silently dropped and the child saw EBADF where GNU's fork hands
//! it the device itself.
//!
//! GNU's permanent `exec 3>/dev/full` installs the OPENED device in
//! the shell's own descriptor table (redir.c:237 do_redirections ->
//! redir.c:725 redir_open) and every forked child inherits that
//! descriptor (execute_cmd.c make_child): its writes fail ENOSPC
//! inside the child, which diagnoses itself and exits nonzero.
//!
//! Expectations byte-verified against GNU bash 5.3.20 with
//! script-file probes (oracle and rubash driving the same host
//! /bin/sh produce byte-identical output). The child's own
//! write-error phrasing belongs to the host's /bin/sh (bash-as-sh:
//! "No space left on device"; dash renders the same ENOSPC as
//! "I/O error"), so the pins assert the invariant shape: a child
//! write-failure report is present, the EBADF report is absent,
//! and stdout / status are exact (the same environment caveat as
//! the #3312 cells in issue3311_exec_child_fd_delivery).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

/// The failure must be the child's own write report on the
/// inherited device, never an EBADF for a descriptor the shell
/// dropped on the floor. The phrasing is the host /bin/sh's own:
/// bash-as-sh says "No space left on device", dash renders the
/// same ENOSPC write failure as "I/O error" — both prove the
/// child held the descriptor and its write hit the device.
fn assert_child_enospc(stderr: &str) {
    assert!(
        stderr.contains("No space left on device") || stderr.contains("I/O error"),
        "child must diagnose its own failed write on the device, stderr: {stderr}"
    );
    assert!(
        !stderr.contains("Bad file descriptor"),
        "the child must hold the descriptor, stderr: {stderr}"
    );
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue3997-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Run `body` as a script FILE under rubash with cwd = dir.
/// Returns (stdout, stderr, status).
fn rubash_script(dir: &Path, body: &str) -> (String, String, Option<i32>) {
    let script = dir.join("case.sh");
    fs::write(&script, body).expect("write script fixture");
    let output = Command::new(RUBASH)
        .arg(&script)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Headline: a standing `exec 3>/dev/full` binding reaches a plain
/// external child at its slot number.
#[cfg(unix)]
#[test]
fn issue3997_standing_full_binding_reaches_external_child() {
    let dir = scratch("headline");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        "exec 3>/dev/full\n/bin/sh -c 'echo f3 >&3'\necho \"child rc=$?\"\n",
    );
    assert_eq!(stdout, "child rc=1\n");
    assert_child_enospc(&stderr);
}

/// The subshell binding form from the report: the group's own
/// `3>/dev/full` redirect is the child's ambient table.
#[cfg(unix)]
#[test]
fn issue3997_subshell_binding_reaches_external_child() {
    let dir = scratch("subshell-bind");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        "( exec /bin/sh -c 'echo f3 >&3' ) 3>/dev/full\necho \"child rc=$?\"\n",
    );
    assert_eq!(stdout, "child rc=1\n");
    assert_child_enospc(&stderr);
}

/// A plain subshell inherits the standing binding through the
/// respawn transport, and its own external child receives it.
#[cfg(unix)]
#[test]
fn issue3997_subshell_inherits_standing_binding() {
    let dir = scratch("subshell-plain");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        "exec 3>/dev/full\n( /bin/sh -c 'echo f3 >&3'; echo \"inner rc=$?\" )\n",
    );
    assert_eq!(stdout, "inner rc=1\n");
    assert_child_enospc(&stderr);
}

/// The append form of the binding delivers the same device.
#[cfg(unix)]
#[test]
fn issue3997_append_binding_reaches_external_child() {
    let dir = scratch("append");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        "exec 3>>/dev/full\n/bin/sh -c 'echo f3 >&3'\necho \"child rc=$?\"\n",
    );
    assert_eq!(stdout, "child rc=1\n");
    assert_child_enospc(&stderr);
}

/// A readwrite binding delivers ONE readwrite description: the
/// child's write fails ENOSPC (not EBADF on a read-only or missing
/// descriptor).
#[cfg(unix)]
#[test]
fn issue3997_readwrite_binding_write_reaches_external_child() {
    let dir = scratch("rw-write");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        "exec 3<>/dev/full\n/bin/sh -c 'echo f3 >&3'\necho \"child rc=$?\"\n",
    );
    assert_eq!(stdout, "child rc=1\n");
    assert_child_enospc(&stderr);
}

/// The readwrite binding's read face: the child reads the device's
/// zero stream at the same slot.
#[cfg(unix)]
#[test]
fn issue3997_readwrite_binding_read_reaches_external_child() {
    let dir = scratch("rw-read");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        "exec 3<>/dev/full\n/bin/sh -c 'head -c 4 <&3 | od -An -tx1'\necho \"child rc=$?\"\n",
    );
    assert_eq!(stdout, " 00 00 00 00\nchild rc=0\n");
    assert_eq!(stderr, "");
}

/// A pipeline stage is a fork of the shell like any external
/// child: the standing binding reaches it too.
#[cfg(unix)]
#[test]
fn issue3997_pipeline_stage_reaches_external_child() {
    let dir = scratch("pipeline");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        "exec 3>/dev/full\n/bin/sh -c 'echo f3 >&3' | cat\necho \"stage rc=${PIPESTATUS[0]}\"\n",
    );
    assert_eq!(stdout, "stage rc=1\n");
    assert_child_enospc(&stderr);
}

/// A trivial command-substitution body (the shortcut spawner)
/// delivers the standing binding like the full path does.
#[cfg(unix)]
#[test]
fn issue3997_comsub_shortcut_reaches_external_child() {
    let dir = scratch("comsub");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        "exec 3>/dev/full\nx=$(/bin/sh -c 'echo f3 >&3' 2>/dev/null; echo \"in rc=$?\")\necho \"[$x]\"\n",
    );
    assert_eq!(stdout, "[in rc=1]\n");
    assert_eq!(stderr, "");
}

/// A process-substitution consumer is forked with the shell's
/// whole ambient table: the binding reaches the consumer's own
/// external child.
#[cfg(unix)]
#[test]
fn issue3997_procsub_consumer_reaches_external_child() {
    let dir = scratch("procsub");
    let (stdout, _, _) = rubash_script(
        &dir,
        "exec 3>/dev/full\ncat <( /bin/sh -c 'echo f3 >&3' 2>&1; echo \"body rc=$?\" )\n",
    );
    assert!(
        stdout.contains("No space left on device") || stdout.contains("I/O error"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("body rc=1"), "stdout: {stdout}");
    assert!(!stdout.contains("Bad file descriptor"), "stdout: {stdout}");
}

/// A background child respawns with the ambient table: the
/// binding reaches its external child.
#[cfg(unix)]
#[test]
fn issue3997_background_child_reaches_external_child() {
    let dir = scratch("background");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        "exec 3>/dev/full\n{ /bin/sh -c 'echo f3 >&3'; echo \"bg rc=$?\"; } &\nwait\n",
    );
    assert_eq!(stdout, "bg rc=1\n");
    assert_child_enospc(&stderr);
}

/// A higher slot delivers identically — nothing is special about
/// the number 3.
#[cfg(unix)]
#[test]
fn issue3997_higher_slot_reaches_external_child() {
    let dir = scratch("slot7");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        "exec 7>/dev/full\n/bin/sh -c 'echo f7 >&7'\necho \"child rc=$?\"\n",
    );
    assert_eq!(stdout, "child rc=1\n");
    assert_child_enospc(&stderr);
}

/// Control: the same shape onto a regular file lands and
/// succeeds, before and after the fix.
#[cfg(unix)]
#[test]
fn issue3997_file_binding_still_lands() {
    let dir = scratch("control-file");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        "exec 3>reg.txt\n/bin/sh -c 'echo f3 >&3'\necho \"child rc=$? f=[$(cat reg.txt)]\"\n",
    );
    assert_eq!(stdout, "child rc=0 f=[f3]\n");
    assert_eq!(stderr, "");
}

/// Control: the command's own close of the slot wins over the
/// ambient delivery — the child must see no descriptor.
#[cfg(unix)]
#[test]
fn issue3997_own_close_wins_over_ambient_delivery() {
    let dir = scratch("control-close");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        "exec 3>/dev/full\n/bin/sh -c 'echo f3 >&3' 3>&-\necho \"child rc=$?\"\n",
    );
    assert_eq!(stdout, "child rc=2\n");
    assert!(
        stderr.contains("Bad file descriptor"),
        "the closed slot must stay closed, stderr: {stderr}"
    );
    assert!(
        !stderr.contains("No space left on device"),
        "the device must not be delivered, stderr: {stderr}"
    );
}

/// Control: the command's own rebind of the slot wins over the
/// ambient delivery — the payload lands in the file.
#[cfg(unix)]
#[test]
fn issue3997_own_rebind_wins_over_ambient_delivery() {
    let dir = scratch("control-rebind");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        "exec 3>/dev/full\n/bin/sh -c 'echo f3 >&3' 3>reg.txt\necho \"child rc=$? f=[$(cat reg.txt)]\"\n",
    );
    assert_eq!(stdout, "child rc=0 f=[f3]\n");
    assert_eq!(stderr, "");
}

/// Control: the in-shell face is unchanged — a builtin write to
/// the standing binding still fails through the shell's own
/// ENOSPC shape.
#[cfg(unix)]
#[test]
fn issue3997_inshell_write_face_unchanged() {
    let dir = scratch("control-inshell");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        "exec 3>/dev/full\necho x >&3\necho \"inshell rc=$?\"\n",
    );
    assert_eq!(stdout, "inshell rc=1\n");
    assert!(
        stderr.contains("No space left on device"),
        "stderr: {stderr}"
    );
}

/// Captain graft guard: a compound bracket's undo save of the
/// standing /dev/full binding is materialized into the fd table
/// at a slot >= 10 for the bracket's duration (rubash#3991), but
/// it is scope restore state, not a live binding — GNU's save
/// descriptor is close-on-exec and never reaches the child. The
/// ambient delivery must skip registered save slots
/// (fd_is_walk_save_slot); the unfixed #4012 traversal recorded
/// a device pin for the save clone and the child saw /dev/full
/// at fd 10. The child's own glob of /proc/self/fd is the
/// witness: its directory descriptor takes the lowest free
/// number (4 with fd 3 bound, 3 with fd 3 closed), and no slot
/// >= 10 may appear. Byte-verified against GNU bash 5.3.20 and
/// pristine c885c5a5 (which passes by accident — the symbolic
/// endpoint was dropped before it could be delivered).
#[cfg(unix)]
#[test]
fn issue3997_bracket_save_slot_not_delivered_rebind() {
    let dir = scratch("save-rebind");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        "exec 3>/dev/full\n{ /bin/sh -c 'echo /proc/self/fd/*'; } 3>cap.txt\n",
    );
    assert_eq!(
        stdout,
        "/proc/self/fd/0 /proc/self/fd/1 /proc/self/fd/2 /proc/self/fd/3 /proc/self/fd/4\n"
    );
    assert_eq!(stderr, "");
}

/// The close form of the same guard: the bracket closes fd 3,
/// its undo save still sits in the table during the body, and
/// the child must see neither the device at fd 3 nor the save
/// clone at a high slot.
#[cfg(unix)]
#[test]
fn issue3997_bracket_save_slot_not_delivered_close() {
    let dir = scratch("save-close");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        "exec 3>/dev/full\n{ /bin/sh -c 'echo /proc/self/fd/*'; } 3>&-\n",
    );
    assert_eq!(
        stdout,
        "/proc/self/fd/0 /proc/self/fd/1 /proc/self/fd/2 /proc/self/fd/3\n"
    );
    assert_eq!(stderr, "");
}

/// Captain graft guard: a simple command closing the only
/// stamped slot leaves a delivery plan whose only content is
/// the revoked carrier. The retiring pre_exec closure must
/// still run for a dead-only plan, or the child inherits
/// /dev/full at the carrier's own number (fd 10) — GNU
/// delivers nothing there (captain save-slot probe sv2/sv11).
#[cfg(unix)]
#[test]
fn issue3997_close_only_slot_no_carrier_leak_full() {
    let dir = scratch("close-carrier-full");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        "exec 3>/dev/full\n/bin/sh -c 'echo /proc/self/fd/*' 3>&-\n",
    );
    assert_eq!(
        stdout,
        "/proc/self/fd/0 /proc/self/fd/1 /proc/self/fd/2 /proc/self/fd/3\n"
    );
    assert_eq!(stderr, "");
}

/// The plain-file form of the same guard (captain probe
/// sv2b/sv12): the stamp carrier of a closed file slot is the
/// same dead-only plan shape.
#[cfg(unix)]
#[test]
fn issue3997_close_only_slot_no_carrier_leak_file() {
    let dir = scratch("close-carrier-file");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        "exec 3>cap.txt\n/bin/sh -c 'echo /proc/self/fd/*' 3>&-\n",
    );
    assert_eq!(
        stdout,
        "/proc/self/fd/0 /proc/self/fd/1 /proc/self/fd/2 /proc/self/fd/3\n"
    );
    assert_eq!(stderr, "");
}
