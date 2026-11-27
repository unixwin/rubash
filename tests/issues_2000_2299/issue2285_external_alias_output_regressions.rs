//! Issue unixwin/rubash#2285: the external-command stdio faces
//! mis-served an fd-alias PATH spelling (`> /dev/fd/N`) whose
//! endpoint is read-bound: the stdio plan declined (a read-bound
//! binding has no write endpoint to dup) and the legacy stdio arms
//! fired on no branch, so the child inherited the shell's real
//! stdout and the alias target was never written.
//!
//! GNU: with HAVE_DEV_FD defined, redir.c's RF_DEVFD dup special
//! case (redir.c:602-629) does not compile, so an alias spelling
//! under an output instruction takes the plain open in redir_open
//! (redir.c:725, via do_redirections at redir.c:237) — a FRESH open
//! of the resolved regular file, truncate or append per the
//! instruction, noclobber-refused under `set -C`, regardless of the
//! source endpoint's direction or offset. (The issue text's
//! control claim — that a `6<>file` source keeps "dup shared-offset
//! semantics" on this face — is falsified by the oracle: p3/p4
//! below truncate exactly like the read-bound case. The `>&N` DUP
//! spelling is the shared-offset one; see the p7 control.)
//! In-process faces already resolved this way since rubash#2275;
//! these pins cover the external stdio plan and the legacy stdio
//! arms. Expectations are the GNU Bash 5.3.20 outputs of
//! probes/oracle-pins.sh (cluster workdir), run against the
//! vendored oracle build. Pins run rubash as a subprocess and
//! observe stdout/stderr/files only.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SEED: &str = "alpha\nbeta\ngamma\n";

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2285-{tag}-{}", std::process::id()));
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

/// Run `script` as rubash's stdin inside `dir` (seed pre-written).
fn run_script(dir: &Path, script: &str) -> (String, String, Option<i32>) {
    std::fs::write(dir.join("seed"), SEED).expect("write seed");
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

fn read_file(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).expect("read back file")
}

/// The issue's reproducer: a read-bound alias under an external
/// command's `>` must fresh-open (truncate + write) the target —
/// the bytes must NOT escape onto the shell's real stdout, and the
/// seed's prior contents must be replaced, not preserved.
#[test]
fn issue2285_read_bound_alias_output_truncates_target() {
    let dir = test_dir("read-bound");
    let (stdout, stderr, code) =
        run_script(&dir, "exec 6<seed\n/bin/echo X > /dev/fd/6\ncat seed\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "X\n", "stderr: {stderr}");
    assert_eq!(read_file(&dir, "seed"), "X\n");
}

/// Append form: fresh open with O_APPEND — prior contents preserved
/// and the new bytes land after them.
#[test]
fn issue2285_read_bound_alias_append_preserves_target() {
    let dir = test_dir("append");
    let (stdout, stderr, code) =
        run_script(&dir, "exec 6<seed\n/bin/echo Y >> /dev/fd/6\ncat seed\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, format!("{SEED}Y\n"), "stderr: {stderr}");
    assert_eq!(read_file(&dir, "seed"), format!("{SEED}Y\n"));
}

/// A read-write source endpoint does NOT change the shape: the
/// alias PATH spelling still fresh-opens (the issue text's
/// shared-offset control claim is oracle-falsified; the oracle
/// truncates even after the source offset was advanced by a read).
#[test]
fn issue2285_readwrite_endpoint_alias_output_still_fresh() {
    let dir = test_dir("readwrite");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 6<>seed\nIFS= read -r -u 6 _\n/bin/echo X > /dev/fd/6\ncat seed\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "X\n", "stderr: {stderr}");
    assert_eq!(read_file(&dir, "seed"), "X\n");
}

/// A write-bound source endpoint likewise: `B > /dev/fd/7` replaces
/// what the earlier `>&7` dup write put in the file.
#[test]
fn issue2285_write_bound_endpoint_alias_output_still_fresh() {
    let dir = test_dir("write-bound");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 7>out\necho A >&7\n/bin/echo B > /dev/fd/7\ncat out\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "B\n", "stderr: {stderr}");
    assert_eq!(read_file(&dir, "out"), "B\n");
}

/// Noclobber parity with the in-process face: under `set -C` the
/// fresh open refuses with the alias spelling in the diagnostic,
/// the command does not run, and the target is untouched.
#[test]
fn issue2285_noclobber_refuses_and_preserves_target() {
    let dir = test_dir("noclobber");
    let (stdout, stderr, code) = run_script(
        &dir,
        "set -C\nexec 6<seed\n/bin/echo X > /dev/fd/6\necho rc=$?\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert!(
        stderr.contains("/dev/fd/6: cannot overwrite existing file"),
        "stderr: {stderr}"
    );
    assert_eq!(read_file(&dir, "seed"), SEED);
}

/// fd-2 form: an external command's stderr redirected through a
/// read-bound alias lands in the target file.
#[test]
fn issue2285_stderr_alias_output_writes_target() {
    let dir = test_dir("stderr");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 6<seed\n/bin/sh -c 'echo E2 >&2' 2>/dev/fd/6\ncat seed\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "E2\n", "stderr: {stderr}");
    assert_eq!(read_file(&dir, "seed"), "E2\n");
}

/// Control: the `>&N` DUP spelling keeps dup semantics (shared
/// offset, no truncate) — the fix must not turn dup spellings into
/// opens. Writing "X\n" at the shared offset 0 overwrites the
/// seed's first two bytes only.
#[test]
fn issue2285_dup_spelling_control_keeps_shared_offset() {
    let dir = test_dir("dup-control");
    let (stdout, stderr, code) = run_script(&dir, "exec 6<>seed\n/bin/echo X >&6\ncat seed\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "X\npha\nbeta\ngamma\n", "stderr: {stderr}");
    assert_eq!(read_file(&dir, "seed"), "X\npha\nbeta\ngamma\n");
}
