//! rubash#4027 — a move (`N>&M-`) whose target was closed at apply
//! time persisted the source's close unconditionally
//! (persist_move_closes, rubash#1235), even when an EARLIER entry of
//! the same redirect list had rebound the source and earned it an
//! undo save:
//!
//!     exec 9>seed.txt
//!     echo a 9>other.txt 8>&9-   (GNU: fd 9 restored to seed.txt
//!     echo b >&9                  after the builtin; rubash: EBADF)
//!
//! GNU do_redirection_internal (redir.c:1138-1237): the move arm adds
//! the source's undo record — add_undo_redirect (redir_fd,
//! r_close_this, -1) at redir.c:1175 — only when the REDIRECTOR is
//! open (the fcntl(redirector, F_GETFD, 0) != -1 test at
//! redir.c:1173). But that only decides whether the MOVE itself
//! registers a save: any earlier entry that rebound the source while
//! it was open already registered its own add_undo_redirect
//! (redir.c:983, redir.c:1296), and cleanup_redirects
//! (execute_cmd.c:507) replays the whole undo chain — so fd 9 comes
//! back to its pre-list binding regardless of the move's own record.
//! rubash's ordered output walk closed the persistent fd whenever the
//! target was closed, ignoring the pending earlier save; the fix
//! tracks the walk's save registrations (fd_walk_register_saves) and
//! skips the persist close for a source the undo chain will restore.
//!
//! Expectations hand-measured against GNU bash 5.3.20 via script
//! files (WSL /usr/local/bin/bash): builtin, compound, function, and
//! `{var}`-adjacent shapes all restore fd 9; the #1235 persist rule
//! still applies when no earlier save exists.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue4027-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn write_script(dir: &Path, name: &str, body: &str) {
    fs::write(dir.join(name), body).expect("write fixture");
}

/// Run a script FILE in `dir` with stdin closed (null).
fn run_script(dir: &Path, name: &str) -> (String, String, Option<i32>) {
    let output = Command::new(RUBASH)
        .arg(dir.join(name))
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

fn file_contents(dir: &Path, name: &str) -> String {
    fs::read_to_string(dir.join(name)).unwrap_or_default()
}

#[test]
fn issue4027_builtin_move_close_restores_saved_source() {
    // The issue's minimal repro on the buffered-builtin route.
    let dir = scratch("builtin");
    write_script(
        &dir,
        "case.sh",
        "exec 9>seed.txt\necho a 9>other.txt 8>&9-\necho b >&9\necho \"rc=$?\"\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "a\nrc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(file_contents(&dir, "seed.txt"), "b\n");
    assert_eq!(file_contents(&dir, "other.txt"), "");
}

#[test]
fn issue4027_builtin_move_close_restores_saved_source_to_file() {
    // Same repro, asserting the RESTORED binding is the pre-list one:
    // `echo b >&9` must land in seed.txt (the undo save's binding),
    // not fail and not reach other.txt.
    let dir = scratch("builtin-restore");
    write_script(
        &dir,
        "case.sh",
        "exec 9>seed.txt\nprintf 'S\\n' >&9\necho a 9>other.txt 8>&9-\n\
         echo b >&9\necho \"rc=$?\"\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "a\nrc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(file_contents(&dir, "seed.txt"), "S\nb\n");
    assert_eq!(file_contents(&dir, "other.txt"), "");
}

#[test]
fn issue4027_compound_move_close_restores_saved_source() {
    // The same list on a `{ ...; }` compound.
    let dir = scratch("compound");
    write_script(
        &dir,
        "case.sh",
        "exec 9>seed.txt\nprintf 'S\\n' >&9\n{ echo a; } 9>other.txt 8>&9-\n\
         echo b >&9\necho \"rc=$?\"\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "a\nrc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(file_contents(&dir, "seed.txt"), "S\nb\n");
    assert_eq!(file_contents(&dir, "other.txt"), "");
}

#[test]
fn issue4027_function_move_close_restores_saved_source() {
    // The same list on a function call.
    let dir = scratch("function");
    write_script(
        &dir,
        "case.sh",
        "f() { echo inf; }\nexec 9>seed.txt\nf 9>other.txt 8>&9-\n\
         echo b >&9\necho \"rc=$?\"\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "inf\nrc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(file_contents(&dir, "seed.txt"), "b\n");
    assert_eq!(file_contents(&dir, "other.txt"), "");
}

#[test]
fn issue4027_move_close_then_later_rebind_still_restores() {
    // `9>o1.txt 8>&9- 9>o2.txt`: the third entry rebinds fd 9 while it
    // is closed, so GNU registers only a close-undo for it — the
    // earliest save (from `9>o1.txt`) still restores fd 9 to
    // seed.txt at command end.
    let dir = scratch("rebind-after");
    write_script(
        &dir,
        "case.sh",
        "exec 9>seed.txt\necho a 9>o1.txt 8>&9- 9>o2.txt\necho b >&9\necho \"rc=$?\"\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "a\nrc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(file_contents(&dir, "seed.txt"), "b\n");
    assert_eq!(file_contents(&dir, "o1.txt"), "");
    assert_eq!(file_contents(&dir, "o2.txt"), "");
}

#[test]
fn issue4027_stdout_stderr_move_shape_restored() {
    // The `1>a 2>&1-` shape: fd 1 was rebound (saved) before the move
    // closed it, so undo restores fd 1 to the process stdout — the
    // echo inside fails (fd 1 closed for the command), the
    // post-command write works.
    let dir = scratch("stdio-shape");
    write_script(
        &dir,
        "case.sh",
        "echo a 1>a.txt 2>&1-\necho \"rc=$?\"\necho b >&1\necho \"rc2=$?\"\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=1\nb\nrc2=0\n", "stdout: {stdout}");
    // fd 2 was bound to a.txt for the command, so the builtin's
    // write-error diagnostic lands in the file, not on stderr.
    assert!(
        file_contents(&dir, "a.txt").contains("Bad file descriptor"),
        "a.txt must hold the diagnostic: {:?}",
        file_contents(&dir, "a.txt")
    );
}

#[test]
fn issue4027_persist_close_still_fires_without_any_save() {
    // rubash#1235 control: `8>&9-` alone (fd 9 bound only ambiently,
    // fd 8 closed) leaves fd 9 closed after the builtin — the
    // persist close must still land when nothing saved the source.
    let dir = scratch("persist-close");
    write_script(
        &dir,
        "case.sh",
        "exec 9>seed.txt\necho a 8>&9-\necho b >&9\necho \"rc=$?\"\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "a\nrc=1\n", "stdout: {stdout}");
    assert!(
        stderr.contains("Bad file descriptor"),
        "the closed-fd write must report EBADF, got: {stderr}"
    );
    assert_eq!(file_contents(&dir, "seed.txt"), "");
}

#[test]
fn issue4027_open_target_move_restores_both() {
    // With fd 8 open at apply time the move registers its own source
    // save AND a target save: undo restores fd 9 to seed.txt AND
    // fd 8 to sink.txt.
    let dir = scratch("open-target");
    write_script(
        &dir,
        "case.sh",
        "exec 9>seed.txt\nexec 8>sink.txt\necho a 9>other.txt 8>&9-\n\
         echo b >&9\necho c >&8\necho \"rc=$?\"\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "a\nrc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(file_contents(&dir, "seed.txt"), "b\n");
    assert_eq!(file_contents(&dir, "sink.txt"), "c\n");
    assert_eq!(file_contents(&dir, "other.txt"), "");
}

#[test]
fn issue4027_printf_face_same_route() {
    // printf rides the same buffered-builtin route.
    let dir = scratch("printf");
    write_script(
        &dir,
        "case.sh",
        "exec 9>seed.txt\nprintf 'PV\\n' 9>other.txt 8>&9-\n\
         echo b >&9\necho \"rc=$?\"\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    // fd 1 is untouched by the list: PV goes to stdout.
    assert_eq!(stdout, "PV\nrc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(file_contents(&dir, "seed.txt"), "b\n");
    assert_eq!(file_contents(&dir, "other.txt"), "");
}
