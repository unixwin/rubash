//! rubash#2552 — a redirect list that GENUINELY closes fd 1 (or 2)
//! through a `{var}` entry (`{v}>&-`, v=1) lost the foreign child's
//! whole drained stream pair after the child exited: the parent had
//! captured both streams (the closed-stdio shape takes the
//! fd-copy capture route), but the post-wait routing walk skipped
//! the fd_var entry wholesale, reported "no output redirect", and
//! the caller's closed-fds branch discarded the drained bytes
//! instead of routing them — the child's fd-2 lines (and, with v=2,
//! its fd-1 lines) never reached the parent's stderr (stdout).
//!
//! GNU applies the list as ONE ordered walk in the child
//! (redir.c:237 do_redirections): `{v}>&-` is r_close_this under
//! REDIR_VARASSIGN (redir.c:1219-1248, the redirector resolved from
//! the variable at redir.c:1244), so the child's fd 1 is closed and
//! everything it still writes to fd 2 is ordinary inherited stderr.
//!
//! Root cause in rubash: the applier only RECORDS a `{var}` close
//! (fd_var_pending_closes) for the spawn delivery instead of closing
//! the descriptor in the shell's own table, and the delivery
//! consumes the records before the post-wait routing walk runs — so
//! the walk's seeded state never showed the close. The fix folds
//! the close into the routing walk at its list position, resolving
//! the variable exactly as the applier did. A static `1>&-` close
//! was never affected (its walk arm models the close directly) and
//! is pinned here as the control.
//!
//! Every test drives a REAL foreign child (/usr/bin/bash) so no
//! rubash respawn/env channel can mask the result. Expectations
//! byte-verified against GNU bash 5.3.20 with script-file probes
//! (cluster workdir probes/: drv2552, v2, close_redirect,
//! builtin_close).

#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");
const FOREIGN: &str = "/usr/bin/bash";

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2552-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    // The issue's reader: its first line writes to fd 1 (closed in
    // the child on the fd-1 shapes, so the write fails inside the
    // child and the child's own diagnostic reports it on fd 2,
    // after the command's `2>/dev/null` is undone); its second line
    // is a plain fd-2 write. Both lines are the drained fd-2 bytes
    // the parent must deliver.
    fs::write(
        dir.join("reader1.sh"),
        "echo hello >&1 2>/dev/null || echo \"fd1-write-failed\"\necho note >&2\n",
    )
    .expect("write reader");
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
fn issue2552_varfd_close_of_fd1_delivers_drained_stderr() {
    let dir = scratch("close-fd1");
    write_script(
        &dir,
        "case.sh",
        &format!("v=1\n{FOREIGN} reader1.sh {{v}}>&-\necho \"rc=$?\"\n"),
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr: {stderr}");
    assert_eq!(
        stderr, "reader1.sh: line 1: echo: write error: Bad file descriptor\nnote\n",
        "the drained fd-2 bytes must reach the parent's stderr whole"
    );
}

#[test]
fn issue2552_static_close_of_fd1_delivers_drained_stderr() {
    // Control: the static spelling was never broken — its walk arm
    // models the close directly.
    let dir = scratch("static-close-fd1");
    write_script(
        &dir,
        "case.sh",
        &format!("{FOREIGN} reader1.sh 1>&-\necho \"rc=$?\"\n"),
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr: {stderr}");
    assert_eq!(
        stderr, "reader1.sh: line 1: echo: write error: Bad file descriptor\nnote\n",
        "stderr: {stderr}"
    );
}

#[test]
fn issue2552_varfd_close_of_fd2_delivers_drained_stdout() {
    // Mirror: closing fd 2 in the child kills the child's own fd-2
    // writes inside the child (its exit status is 1), while its
    // fd-1 bytes drain and must land on the parent's stdout.
    let dir = scratch("close-fd2");
    write_script(
        &dir,
        "case.sh",
        &format!("v=2\n{FOREIGN} reader1.sh {{v}}>&-\necho \"rc=$?\"\n"),
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "hello\nrc=1\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stderr: {stderr}");
}

#[test]
fn issue2552_varfd_close_with_stderr_redirect_still_routes() {
    // The routing walk's real job must survive the fold: with a
    // plain `2>err.txt` in the same list, the drained fd-2 bytes
    // land in the file, exactly as GNU routes them.
    let dir = scratch("close-and-redirect");
    write_script(
        &dir,
        "case.sh",
        &format!("v=1\n{FOREIGN} reader1.sh {{v}}>&- 2>err.txt\necho \"rc=$?\"\n"),
    );
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(
        file_contents(&dir, "err.txt"),
        "reader1.sh: line 1: echo: write error: Bad file descriptor\nnote\n"
    );
}

#[test]
fn issue2552_builtin_varfd_close_keeps_chkwrite_verdict() {
    // Control: the in-process face was never broken — the applier
    // performs a builtin's `{var}` close in the table, so the
    // builtin's own write to the closed fd 1 takes the sh_chkwrite
    // verdict (builtins/common.c:320-334) with status 1.
    let dir = scratch("builtin-close");
    write_script(&dir, "case.sh", "v=1\necho hi {v}>&-\necho \"rc=$?\"\n");
    let (stdout, stderr, code) = run_script(&dir, "case.sh");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert_eq!(
        stderr, "case.sh: line 2: echo: write error: Bad file descriptor\n",
        "stderr: {stderr}"
    );
}
