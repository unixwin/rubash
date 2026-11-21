//! rubash#4000 — in one redirect walk, a move (`N>&M-`) retired the
//! source's FileFd handle while a dup target bound earlier in the
//! same walk still aliased that handle; the flush then wrote to a
//! closed descriptor:
//!
//!     exec 9>seed.txt
//!     echo W10 1>&9 8>&9-        (GNU: W10 lands, rc=0; rubash:
//!                                 `echo: write error: Bad file
//!                                 descriptor`, rc=1)
//!
//! GNU do_redirection_internal (redir.c:1138-1237) performs the dup
//! with `dup2 (redir_fd, redirector)` (redir.c:1193): the redirector
//! gets its OWN descriptor on the same open file description, and the
//! move arm's `close (redir_fd)` (redir.c:1227-1235) closes only the
//! source NUMBER — a `1>&9` taken earlier keeps the description.
//! rubash's ordered output walk modeled the dup as an Rc clone of the
//! source's FileFd, and the move arm's persist close
//! (close_persistent_output_fd, rubash#1235) retired that shared
//! handle because no fd-table sibling referenced it — the walk's
//! SharedFile aliases outlived it in name only. The fix binds the
//! dup an independent kernel duplicate (FileFd::duplicate, the
//! dup_file_endpoints save-dup discipline of rubash#2613/#3706) on
//! persist_move_closes walks.
//!
//! Expectations hand-measured against GNU bash 5.3.20 via script
//! files (the #1235 move-close persistence rules are asserted both
//! ways: closed target -> source stays closed; open target -> the
//! close is undone and the source survives).

#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue4000-{tag}-{}", std::process::id()));
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
fn issue4000_dup_then_move_of_source() {
    // The issue's minimal repro.
    let dir = scratch("repro");
    write_script(
        &dir,
        "case.sh",
        "exec 9>seed.txt\necho W10 1>&9 8>&9-\necho \"rc=$?\"\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(file_contents(&dir, "seed.txt"), "W10\n");
}

#[test]
fn issue4000_dup_then_move_inside_compound() {
    // The same list on a `{ ...; }` compound: the redirects inject
    // into the leaf's walk, so the identical dup+move pair runs
    // there.
    let dir = scratch("compound");
    write_script(
        &dir,
        "case.sh",
        "exec 9>seed.txt\n{ echo WC 1>&9 8>&9-; }\necho \"rc=$?\"\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(file_contents(&dir, "seed.txt"), "WC\n");
}

#[test]
fn issue4000_two_dups_then_move() {
    // Every dup in the list holds its own descriptor on the
    // description; the move retires only fd 9's number.
    let dir = scratch("two-dups");
    write_script(
        &dir,
        "case.sh",
        "exec 9>seed.txt\necho WE 1>&9 2>&9 8>&9-\necho \"rc=$?\"\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(file_contents(&dir, "seed.txt"), "WE\n");
}

#[test]
fn issue4000_chained_moves() {
    // `7>&8-` moves the fd-8 binding (itself a dup of 9's
    // description) on to 7 — each hop binds an independent
    // descriptor, so the cascade of source closes retires nothing
    // the earlier dups still name.
    let dir = scratch("chained");
    write_script(
        &dir,
        "case.sh",
        "exec 9>seed.txt\necho WF 1>&9 8>&9- 7>&8-\necho \"rc=$?\"\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(file_contents(&dir, "seed.txt"), "WF\n");
}

#[test]
fn issue4000_dup_target_shares_the_open_file_offset() {
    // The independent duplicate names the SAME open file
    // description: the dup'd fd's writes continue at the shared
    // offset, exactly like GNU's dup2.
    let dir = scratch("offset");
    write_script(
        &dir,
        "case.sh",
        "exec 9>seed.txt\nprintf 'AB' >&9\necho CD 1>&9 8>&9-\necho \"rc=$?\"\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(file_contents(&dir, "seed.txt"), "ABCD\n");
}

#[test]
fn issue4000_move_close_still_persists_for_closed_target() {
    // rubash#1235 control: `8>&9-` with fd 8 closed at apply time
    // leaves fd 9 closed after the builtin — the persist close must
    // still land; only the dup aliases must survive it.
    let dir = scratch("persist-close");
    write_script(
        &dir,
        "case.sh",
        "exec 9>seed.txt\necho WP 1>&9 8>&9-\necho \"rc=$?\"\n\
         echo AFTER >&9\necho \"rc2=$?\"\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\nrc2=1\n", "stdout: {stdout}");
    assert!(
        stderr.contains("Bad file descriptor"),
        "the closed-fd write must report EBADF, got: {stderr}"
    );
    assert_eq!(file_contents(&dir, "seed.txt"), "WP\n");
}

#[test]
fn issue4000_move_close_undone_when_target_was_open() {
    // The other #1235 half: with fd 8 open at apply time the move's
    // source close IS undone after the builtin, so fd 9 still
    // accepts output afterwards.
    let dir = scratch("undo-close");
    write_script(
        &dir,
        "case.sh",
        "exec 9>seed.txt\nexec 8>/dev/null\necho WU 1>&9 8>&9-\n\
         echo x >&9\necho \"rc=$?\"\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(file_contents(&dir, "seed.txt"), "WU\nx\n");
}

#[test]
fn issue4000_move_first_then_dup_still_fails() {
    // Positionality control: `8>&9-` closes fd 9 at its position, so
    // the later `1>&9` dups a closed descriptor — GNU rejects with
    // EBADF and the command never runs.
    let dir = scratch("move-first");
    write_script(
        &dir,
        "case.sh",
        "exec 9>seed.txt\necho WC 8>&9- 1>&9\necho \"rc=$?\"\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=1\n", "stdout: {stdout} stderr: {stderr}");
    assert!(
        stderr.contains("Bad file descriptor"),
        "stderr must report the closed source fd, got: {stderr}"
    );
    assert_eq!(file_contents(&dir, "seed.txt"), "");
}

#[test]
fn issue4000_dup_then_plain_close_control() {
    // Control: `9>&-` never persisted a close in the walk, so the
    // dup alias always survived it — unchanged behavior.
    let dir = scratch("plain-close");
    write_script(
        &dir,
        "case.sh",
        "exec 9>seed.txt\necho WB 1>&9 9>&-\necho \"rc=$?\"\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(file_contents(&dir, "seed.txt"), "WB\n");
}

#[test]
fn issue4000_other_builtin_dup_then_move() {
    // The defect is in the shared buffered-output walk, not echo's
    // face: printf hits the same route.
    let dir = scratch("printf");
    write_script(
        &dir,
        "case.sh",
        "exec 9>seed.txt\nprintf 'PV\\n' 1>&9 8>&9-\necho \"rc=$?\"\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(file_contents(&dir, "seed.txt"), "PV\n");
}

#[test]
fn issue4000_dev_fd_spelling_dup_then_move() {
    // Captain graft pin (2026-11-21 rebase lane): landing point 3 of
    // the fix — the `/dev/fd/N` spelling open. The plain spelling
    // shapes never reach it (a fresh open of the path serves them,
    // with or without the fix); unlinking the file after
    // `exec 9>seed.txt` makes the fresh open fail, so the walk
    // falls back to fd 9's table endpoint — and on a
    // persist_move_closes walk that binding must be an independent
    // duplicate, or the `8>&9-` move retires the shared handle and
    // the flush fails EBADF. The hardlink keeps the written bytes
    // readable after the unlink. GNU serves the shape (rc=0, bytes
    // land), measured against the 5.3.20 oracle by script file.
    let dir = scratch("dev-fd-spelling");
    write_script(
        &dir,
        "case.sh",
        "exec 9>seed.txt\nln seed.txt keep.txt\nrm seed.txt\n\
         echo WD 1>/dev/fd/9 8>&9-\necho \"rc=$?\"\n",
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(file_contents(&dir, "keep.txt"), "WD\n");
}
