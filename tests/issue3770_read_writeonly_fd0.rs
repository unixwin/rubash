//! rubash#3770: `read` on a write-only fd 0 must report the EBADF
//! diagnostic instead of failing silently.
//!
//! After `exec 0<<A 0>out.txt` fd 0 is open but write-only (#3655
//! retires the heredoc read arm, so the descriptor itself is fine —
//! the read is what fails). GNU reports the hard read failure
//! (builtins/read.def:911 — `read: <fd>: read error: <strerror>`,
//! builtin fails, not plain EOF); rubash failed with the same rc and
//! empty `x` but printed no diagnostic: the fd-0 availability check
//! only consulted `is_closed(0)`, and the read transports returned
//! None without recording a hard error, so the builtin fell through
//! to the silent-EOF arm.
//!
//! Every expectation below is byte-verified against GNU bash 5.3.20
//! (oracle probes ~/workspace/fix-3770/probe-oracle/c1..c8): the
//! failing reads report `rc=1` with `read: <fd>: read error: Bad file
//! descriptor` on stderr; the reverse-order control still reads the
//! heredoc, the `<>` pair still reads, and a real EOF stays silent.

use std::path::PathBuf;
use std::process::Command;

fn run_script(name: &str, script: &str) -> (String, String, Option<i32>, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rubash-issue3770-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create probe directory");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .env_remove("BASH_ENV")
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
        dir,
    )
}

fn read_probe_file(dir: &PathBuf, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).unwrap_or_else(|_| panic!("probe file {name} missing"))
}

#[test]
fn issue_fd0_heredoc_then_truncate_file_diagnoses() {
    // The issue prototype: fd 0 write-only after heredoc-then-file.
    // This is the full expectation of #3768's relaxed J cell
    // (issue_fd0_heredoc_then_truncate_file_wins), which pins the
    // ordering face only and cites #3770 for the diagnostic.
    let (stdout, stderr, code, dir) = run_script(
        "fd0-hd-trunc",
        "exec 0<<A 0>out.txt\n\
         first\n\
         A\n\
         read x; echo \"rc=$? x=$x\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "rc=1 x=\n", "stdout={stdout:?}");
    assert!(
        stderr.contains("read: 0: read error: Bad file descriptor"),
        "write-only fd 0 must be a read error, got: {stderr:?}"
    );
    assert_eq!(read_probe_file(&dir, "out.txt"), "");
}

#[test]
fn issue_read_u14_writeonly_diagnoses() {
    // Control: `read -u 14` on the same write-only shape already
    // diagnoses (its availability check consults is_open_for_read).
    // Guards the -u arm against the fix.
    let (stdout, stderr, code, dir) = run_script(
        "u14-hd-trunc",
        "ulimit -n 16\n\
         exec 14<<A 14>cross.txt\n\
         first\n\
         A\n\
         read -u 14 x; echo \"rc=$? x=$x\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "rc=1 x=\n", "stdout={stdout:?}");
    assert!(
        stderr.contains("read: 14: read error: Bad file descriptor"),
        "write-only fd 14 must be a read error, got: {stderr:?}"
    );
    assert_eq!(read_probe_file(&dir, "cross.txt"), "");
}

#[test]
fn issue_read_u0_writeonly_diagnoses() {
    // `read -u 0` names fd 0 itself: same EBADF diagnostic as a
    // plain `read` on the write-only descriptor.
    let (stdout, stderr, code, dir) = run_script(
        "u0-hd-trunc",
        "exec 0<<A 0>out.txt\n\
         first\n\
         A\n\
         read -u 0 x; echo \"rc=$? x=$x\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "rc=1 x=\n", "stdout={stdout:?}");
    assert!(
        stderr.contains("read: 0: read error: Bad file descriptor"),
        "write-only fd 0 via -u 0 must be a read error, got: {stderr:?}"
    );
    assert_eq!(read_probe_file(&dir, "out.txt"), "");
}

#[test]
fn issue_own_output_redirect_diagnoses() {
    // The command's own redirect makes fd 0 write-only for this
    // read only: GNU applies the redirect first (redir.c:237), then
    // the read fails EBADF.
    let (stdout, stderr, code, dir) =
        run_script("own-out-redir", "read x 0>out2.txt; echo \"rc=$? x=$x\"\n");
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "rc=1 x=\n", "stdout={stdout:?}");
    assert!(
        stderr.contains("read: 0: read error: Bad file descriptor"),
        "own-redirect write-only fd 0 must be a read error, got: {stderr:?}"
    );
    assert_eq!(read_probe_file(&dir, "out2.txt"), "");
}

#[test]
fn issue_reverse_order_heredoc_reads() {
    // Reverse-order control: file-then-heredoc leaves the heredoc
    // read arm in place, so `read` still sees `first`.
    let (stdout, stderr, code, dir) = run_script(
        "fd0-trunc-hd",
        "exec 0>out.txt 0<<A\n\
         first\n\
         A\n\
         read x; echo \"rc=$? x=$x\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "rc=0 x=first\n", "stdout={stdout:?}");
    assert_eq!(stderr, "", "stderr={stderr:?}");
    assert_eq!(read_probe_file(&dir, "out.txt"), "");
}

#[test]
fn issue_readwrite_pair_still_reads() {
    // The `<>` pair is the one legitimate dual-arm install
    // (FdTable::open_readwrite): a read on it still serves.
    let (stdout, stderr, code, _dir) = run_script(
        "rw-pair",
        "printf 'hello\\n' > rw.txt\n\
         exec 14<>rw.txt\n\
         read -u 14 x; echo \"rc=$? x=$x\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "rc=0 x=hello\n", "stdout={stdout:?}");
    assert_eq!(stderr, "", "stderr={stderr:?}");
}

#[test]
fn issue_closed_fd0_diagnoses() {
    // Control: a closed fd 0 already diagnoses via the closed-fd arm
    // (rubash#1477) — the fix must not change its wording.
    let (stdout, stderr, code, _dir) =
        run_script("closed-fd0", "read x <&-; echo \"rc=$? x=$x\"\n");
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "rc=1 x=\n", "stdout={stdout:?}");
    assert!(
        stderr.contains("read: 0: read error: Bad file descriptor"),
        "closed fd 0 must be a read error, got: {stderr:?}"
    );
}

#[test]
fn issue_eof_stays_silent() {
    // Control: a real EOF is not a hard read failure (read.def:949)
    // and must stay silent — the fix must not diagnose EOF.
    let (stdout, stderr, code, _dir) =
        run_script("eof-silent", "read x </dev/null; echo \"rc=$? x=$x\"\n");
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "rc=1 x=\n", "stdout={stdout:?}");
    assert_eq!(stderr, "", "stderr={stderr:?}");
}
