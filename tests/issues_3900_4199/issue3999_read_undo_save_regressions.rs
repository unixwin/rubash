//! Issue unixwin/rubash#3999: the `read` builtin's serve and
//! availability machinery had no undo-save resolution.
//!
//! PR #3998 (c885c5a5d) materialized redirect undo saves as
//! resolvable descriptors in the shared walk space
//! (fd_walk_register_saves): an entry that rebinds an open
//! descriptor earns a save at the lowest free slot at or above
//! SHELL_FD_BASE (10) — a real descriptor GNU's do_redirections
//! walk holds for the list's duration (add_undo_redirect,
//! redir.c:1296). A later entry can dup from the save slot, and
//! `read` validates and serves through sh_validfd
//! (builtins/read.def:377-381) and the redirected descriptor —
//! but rubash's read-side scans only saw ordinary list entries
//! and the persistent fd table, so the save slot read as
//! unavailable (`10: Bad file descriptor`) or served EOF.
//!
//! The fix resolves the save slot through the same walk space at
//! the position GNU judges it: a `<&N` dup source is judged and
//! served at the dup entry's own list position, a `-u N` read at
//! end of list, and the descriptor the save copied is served
//! through the same transports (the fd-table endpoint for a
//! TableSlot binding, the stored entry for a File binding).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue3999-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
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

/// Run `body` as a script inside `dir`.
fn run_in(dir: &Path, body: &str) -> (String, String, Option<i32>) {
    // Forward slashes: a Windows temp path's backslashes would be
    // eaten by the shell's word splitting inside the `cd` argument.
    let dir = dir.to_string_lossy().replace('\\', "/");
    rubash_stdin(&format!("cd {dir} || exit 1\n{body}"))
}

fn seed(dir: &Path) {
    std::fs::write(dir.join("in.txt"), "hello\n").expect("seed in.txt");
    std::fs::write(dir.join("o1.txt"), "other\n").expect("seed o1.txt");
}

#[test]
fn issue3999_dup_after_save_entry_reads_the_save() {
    // The issue's reproducer: `9<o1.txt` earns the undo save of
    // fd 9 (holding in.txt) at slot 10, `0<&10` dups it onto fd 0,
    // and read consumes the saved description — GNU rc=0 x=hello.
    let dir = test_dir("dup-after");
    seed(&dir);
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9<in.txt\n\
         read x 9<o1.txt 0<&10\n\
         echo \"rc=$? x=$x\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0 x=hello\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stdout: {stdout}");
}

#[test]
fn issue3999_dup_before_save_entry_is_bad_fd() {
    // Position matters: `0<&10` runs before any entry earned a
    // save, so fd 10 is genuinely closed — GNU's dup2 fails with
    // `10: Bad file descriptor` and the command never runs.
    let dir = test_dir("dup-before");
    seed(&dir);
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9<in.txt\n\
         read y 0<&10 9<o1.txt\n\
         echo \"rc=$? y=$y\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=1 y=\n", "stderr: {stderr}");
    assert!(
        stderr.contains("10: Bad file descriptor"),
        "stderr must name the bad dup source; got {stderr:?}"
    );
}

#[test]
fn issue3999_u_flag_reads_the_save_slot_directly() {
    // `read -u 10` names the save slot itself: GNU's sh_validfd
    // sees the walk's live descriptor and builtin_read consumes
    // the saved description — rc=0 z=hello.
    let dir = test_dir("u-save");
    seed(&dir);
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9<in.txt\n\
         read -u 10 z 9<o1.txt\n\
         echo \"rc=$? z=$z\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0 z=hello\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stdout: {stdout}");
}

#[test]
fn issue3999_close_after_the_dup_keeps_the_duplicate() {
    // `10<&-` closes the save slot AFTER `0<&10` already dup'd it:
    // the fd-0 duplicate keeps the description and the read still
    // succeeds — GNU rc=0 w=hello.
    let dir = test_dir("close-after");
    seed(&dir);
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9<in.txt\n\
         read w 9<o1.txt 0<&10 10<&-\n\
         echo \"rc=$? w=$w\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0 w=hello\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stdout: {stdout}");
}

#[test]
fn issue3999_u_flag_on_slot_closed_later_is_invalid() {
    // The mirror image: `read -u 10` is judged at END of list, so
    // the later `10<&-` closes the save before the builtin runs —
    // GNU reports `read: 10: invalid file descriptor` rc=1.
    let dir = test_dir("u-closed");
    seed(&dir);
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9<in.txt\n\
         read -u 10 v 9<o1.txt 10<&-\n\
         echo \"rc=$? v=$v\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=1 v=\n", "stderr: {stderr}");
    assert!(
        stderr.contains("10: invalid file descriptor"),
        "stderr must report the invalid descriptor; got {stderr:?}"
    );
}

#[test]
fn issue3999_dup_chain_through_the_save_slot() {
    // `3<&10` dups the save onto fd 3 and `read -u 3` follows the
    // own-list duplicate chain back to the walk's save binding —
    // GNU rc=0 x=hello.
    let dir = test_dir("dup-chain");
    seed(&dir);
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9<in.txt\n\
         read -u 3 x 9<o1.txt 3<&10\n\
         echo \"rc=$? x=$x\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0 x=hello\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stdout: {stdout}");
}

#[test]
fn issue3999_move_dup_from_the_save_slot() {
    // The move form `0<&10-` dups the save onto fd 0 and closes
    // slot 10 — the read still consumes the saved description,
    // GNU rc=0 x=hello.
    let dir = test_dir("move-dup");
    seed(&dir);
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9<in.txt\n\
         read x 9<o1.txt 0<&10-\n\
         echo \"rc=$? x=$x\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0 x=hello\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stdout: {stdout}");
}

#[test]
fn issue3999_later_bind_shadows_the_save_slot() {
    // A later entry binding the save slot for real wins over the
    // save (the walk clobbers it): `10<o1.txt` rebinds slot 10, so
    // `read -u 10` reads o1.txt's "other", not the save's "hello".
    let dir = test_dir("shadow");
    seed(&dir);
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9<in.txt\n\
         read -u 10 x 9<o1.txt 10<o1.txt\n\
         echo \"rc=$? x=$x\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0 x=other\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stdout: {stdout}");
}

#[test]
fn issue3999_two_saves_second_dup_reads_first_save() {
    // Two rebinds earn saves at 10 and 11; `0<&11` dups the SECOND
    // save — the description fd 8 held (in2.txt) — so the read
    // consumes in2.txt's record, GNU rc=0 u=world.
    let dir = test_dir("two-saves");
    std::fs::write(dir.join("in.txt"), "hello\n").expect("seed in.txt");
    std::fs::write(dir.join("in2.txt"), "world\n").expect("seed in2.txt");
    std::fs::write(dir.join("o1.txt"), "").expect("seed o1.txt");
    std::fs::write(dir.join("o2.txt"), "").expect("seed o2.txt");
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9<in.txt\n\
         exec 8<in2.txt\n\
         read u 9<o1.txt 8<o2.txt 0<&11\n\
         echo \"rc=$? u=$u\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0 u=world\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stdout: {stdout}");
}

#[test]
fn issue3999_save_slot_with_no_earning_entry_stays_bad_fd() {
    // Negative control: nothing earned a save at slot 12, so the
    // dup still fails exactly like GNU.
    let dir = test_dir("no-save");
    seed(&dir);
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9<in.txt\n\
         read y 0<&12 9<o1.txt\n\
         echo \"rc=$? y=$y\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=1 y=\n", "stderr: {stderr}");
    assert!(
        stderr.contains("12: Bad file descriptor"),
        "stderr must name the bad dup source; got {stderr:?}"
    );
}

/// Captain graft pins for PR #4033 (review-4033.md named gaps).
/// Each pin is fail-first on pristine eb1ce9f2: the shapes below
/// are red there with the signature recorded in the comment, and
/// green on the #4033 tree. Probe corpus: lane3999-probes.md.

#[test]
fn issue3999_write_endpoint_save_u_flag_is_read_error() {
    // The save slot copies a WRITE endpoint: GNU resolves slot 10
    // (fd 9's old write binding) and `read -u 10` fails at the
    // READ layer — `read: 10: read error: Bad file descriptor`,
    // rc=1 — not at the redirect layer. Pristine fails the same
    // shape with `read: 10: invalid file descriptor` (the save
    // slot reads as unavailable), so the stderr wording below is
    // the fail-first discriminator, not just rc/stdout.
    let dir = test_dir("write-u");
    std::fs::write(dir.join("out.txt"), "w0\n").expect("seed out.txt");
    std::fs::write(dir.join("o2.txt"), "w2\n").expect("seed o2.txt");
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9>out.txt\n\
         read -u 10 x 9>o2.txt\n\
         echo \"rc=$? x=$x\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=1 x=\n", "stderr: {stderr}");
    assert!(
        stderr.contains("read: 10: read error: Bad file descriptor"),
        "stderr must report the read-layer error; got {stderr:?}"
    );
}

#[test]
fn issue3999_write_endpoint_save_fd0_is_read_error() {
    // Same write-endpoint save through the fd-0 arm: GNU resolves
    // the save, `0<&10` dups the write endpoint onto fd 0, and
    // the read itself fails — `read: 0: read error: Bad file
    // descriptor`, rc=1. Pristine fails one layer earlier with a
    // bare `10: Bad file descriptor` on the dup itself.
    let dir = test_dir("write-fd0");
    std::fs::write(dir.join("out.txt"), "w0\n").expect("seed out.txt");
    std::fs::write(dir.join("o2.txt"), "w2\n").expect("seed o2.txt");
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9>out.txt\n\
         read x 9>o2.txt 0<&10\n\
         echo \"rc=$? x=$x\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=1 x=\n", "stderr: {stderr}");
    assert!(
        stderr.contains("read: 0: read error: Bad file descriptor"),
        "stderr must report the read-layer error; got {stderr:?}"
    );
}

#[test]
fn issue3999_heredoc_save_source_reads_the_save() {
    // A heredoc rebind earns the save of fd 9 (in.txt) at slot 10;
    // `0<&10` dups it onto fd 0 and read consumes the saved
    // description — GNU rc=0 x=hello. Pristine has no save
    // resolution here: rc=1 `10: Bad file descriptor`.
    let dir = test_dir("heredoc-save");
    seed(&dir);
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec 9<in.txt\n\
         read x 9<<<hi 0<&10\n\
         echo \"rc=$? x=$x\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0 x=hello\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stdout: {stdout}");
}

#[test]
fn issue3999_coproc_save_source_reads_the_save() {
    // A coproc read end bound to fd 9 earns the save at slot 10
    // when re-bound; `0<&10` dups it onto fd 0 and read consumes
    // the coproc's line — GNU rc=0 x=coproc-hello. Pristine:
    // rc=1 `10: Bad file descriptor`.
    let dir = test_dir("coproc-save");
    std::fs::write(dir.join("o1.txt"), "other\n").expect("seed o1.txt");
    let (stdout, stderr, code) = run_in(
        &dir,
        "coproc CP { printf 'coproc-hello\\n'; sleep 0.2; }\n\
         exec 9<&${CP[0]}\n\
         read x 9<o1.txt 0<&10\n\
         echo \"rc=$? x=$x\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0 x=coproc-hello\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stdout: {stdout}");
}
