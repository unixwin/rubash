//! Issue rubash#3980 (#3941 residual): a builtin inside a procsub
//! body writing to an inherited read-only fd — `cat f1 >&3`,
//! `cat <<EOF >&3`, `echo`/`printf >&3` inside `<(...)` / `>(...)`
//! when fd 3 is an open read-only slot — must surface the write
//! error GNU emits (`cat: write error: Bad file descriptor`), not
//! fail silently.
//!
//! GNU model (vendored third_party/bash): the procsub body is a
//! forked child (subst.c:6362 process_substitute); `>&N` is a plain
//! dup2 of fd N's open file description (redir.c:1193
//! do_redirection_internal r_duplicating_output) which succeeds for
//! ANY open descriptor — access mode is not consulted — so the
//! failure surfaces at the command's own write (coreutils cat.c
//! `cat: write error: Bad file descriptor`, rc 1; the detached
//! procsub body's status is unobservable, parent rc stays 0). A
//! CLOSED fd fails the dup at setup instead (`N: Bad file
//! descriptor`, redir.c redirection_error).
//!
//! Rubash's emulated `cat` (external_file_builtins) buffered its
//! output through write_cat_output, whose `>&N` dup-spelling target
//! fell into write_dev_stdio_redirect_output — the /dev/fd/N
//! ALIAS-open face — which fresh-opened the source's path for
//! writing: a read-only fd's file was silently truncated and
//! rewritten, and append-bound descriptions lost their offset.
//! write_cat_dup_fd_output now serves the dup2 face: writable
//! sources write through the shared endpoint, write-less sources
//! fail the write EBADF.
//!
//! The tests open fd 3 INSIDE the script (`exec 3<seed`), which
//! produces the same fd-table shape as a parent-inherited
//! descriptor (FdReadEndpoint::File, no write endpoint) on every
//! platform — no pre_exec needed. Expected outputs verified
//! against WSL GNU Bash 5.3.x (script files).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3980-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

/// Write `script` as case.sh inside `dir` and run rubash on it with
/// `dir` as cwd, so the script's relative filenames (f1, fd3src,
/// out.txt) resolve without path-spelling concerns on any platform.
fn run_script(dir: &Path, script: &str) -> Run {
    let script_path = dir.join("case.sh");
    fs::write(&script_path, script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn seed(dir: &Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, contents).expect("write seed file");
    path
}

fn file_contents(path: &Path) -> String {
    fs::read_to_string(path).expect("read result file")
}

#[test]
fn issue3980_cat_plain_ro_fd_reports_write_error() {
    // Direct (non-procsub) control: `cat f1 >&3` with fd 3 open
    // read-only. GNU: dup2 succeeds, cat's write fails EBADF —
    // `cat: write error: Bad file descriptor`, rc 1, and the
    // descriptor's file is NOT touched (the pre-fix alias face
    // truncated and rewrote it).
    let dir = tempdir("plain");
    seed(&dir, "f1", "cat-data\n");
    let fd3src = seed(&dir, "fd3src", "seed3\n");
    let run = run_script(&dir, "exec 3<fd3src\ncat f1 >&3\necho rc=$?\n");
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "rc=1\n", "stderr: {}", run.stderr);
    assert!(
        run.stderr.contains("cat: write error: Bad file descriptor"),
        "stderr: {}",
        run.stderr
    );
    assert_eq!(file_contents(&fd3src), "seed3\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue3980_cat_heredoc_ro_fd_reports_write_error() {
    // The heredoc arm duplicates the same redirect_out serve inline
    // in external_cat — `cat <<EOF >&3` must fail identically.
    let dir = tempdir("heredoc");
    let fd3src = seed(&dir, "fd3src", "seed3\n");
    let run = run_script(
        &dir,
        "exec 3<fd3src\ncat <<EOF >&3\nbody\nEOF\necho rc=$?\n",
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "rc=1\n", "stderr: {}", run.stderr);
    assert!(
        run.stderr.contains("cat: write error: Bad file descriptor"),
        "stderr: {}",
        run.stderr
    );
    assert_eq!(file_contents(&fd3src), "seed3\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue3980_cat_move_dup_ro_fd_reports_write_error() {
    // `>&3-` (dup + move-close) shares the dup2 face: the write
    // still fails EBADF on the read-only description.
    let dir = tempdir("move");
    seed(&dir, "f1", "cat-data\n");
    let fd3src = seed(&dir, "fd3src", "seed3\n");
    let run = run_script(&dir, "exec 3<fd3src\ncat f1 >&3-\necho rc=$?\n");
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "rc=1\n", "stderr: {}", run.stderr);
    assert!(
        run.stderr.contains("cat: write error: Bad file descriptor"),
        "stderr: {}",
        run.stderr
    );
    assert_eq!(file_contents(&fd3src), "seed3\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue3980_cat_procsub_input_ro_fd_reports_write_error() {
    // The issue's headline shape: `<(cat f1 >&3)` — the procsub
    // body is a detached child; its write error is diagnosed at
    // the construction line while the parent status stays clean
    // (GNU subst.c:6362 — the body status is unobservable).
    let dir = tempdir("procsub-in");
    seed(&dir, "f1", "cat-data\n");
    let fd3src = seed(&dir, "fd3src", "seed3\n");
    let run = run_script(
        &dir,
        "exec 3<fd3src\ncat <(cat f1 >&3)\nsleep 0.5\necho rc=$?\n",
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "rc=0\n", "stderr: {}", run.stderr);
    assert!(
        run.stderr.contains("cat: write error: Bad file descriptor"),
        "stderr: {}",
        run.stderr
    );
    assert_eq!(file_contents(&fd3src), "seed3\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue3980_cat_procsub_output_ro_fd_reports_write_error() {
    // The `>(...)` twin: same detached-body contract.
    let dir = tempdir("procsub-out");
    seed(&dir, "f1", "cat-data\n");
    let fd3src = seed(&dir, "fd3src", "seed3\n");
    let run = run_script(
        &dir,
        "exec 3<fd3src\necho x > >(cat f1 >&3)\nsleep 0.5\necho rc=$?\n",
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "rc=0\n", "stderr: {}", run.stderr);
    assert!(
        run.stderr.contains("cat: write error: Bad file descriptor"),
        "stderr: {}",
        run.stderr
    );
    assert_eq!(file_contents(&fd3src), "seed3\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue3980_cat_writable_fd3_shares_description() {
    // Control: a WRITABLE fd 3 takes the bytes through the shared
    // open file description — `exec 3>>out` + an earlier write
    // leaves cat's bytes appended at the shared offset, never a
    // fresh truncate (the alias face's old behavior).
    let dir = tempdir("writable");
    seed(&dir, "f1", "cat-data\n");
    let out = dir.join("out3.txt");
    let run = run_script(
        &dir,
        "exec 3>>out3.txt\necho seed >&3\ncat f1 >&3\necho rc=$?\n",
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "rc=0\n", "stderr: {}", run.stderr);
    assert_eq!(run.stderr, "", "stderr: {}", run.stderr);
    assert_eq!(file_contents(&out), "seed\ncat-data\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue3980_cat_alias_spelling_still_reopens() {
    // Control: `/dev/fd/N` is the ALIAS-open face (redir.c:725
    // redir_open), NOT a dup2 — `cat f1 >/dev/fd/3` with fd 3
    // read-only bound to fd3src fresh-opens fd3src for writing and
    // replaces its contents, exactly like GNU.
    let dir = tempdir("alias");
    seed(&dir, "f1", "cat-data\n");
    let fd3src = seed(&dir, "fd3src", "seed3\n");
    let run = run_script(&dir, "exec 3<fd3src\ncat f1 >/dev/fd/3\necho rc=$?\n");
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "rc=0\n", "stderr: {}", run.stderr);
    assert_eq!(file_contents(&fd3src), "cat-data\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue3980_cat_closed_fd3_bad_fd_unchanged() {
    // Closed-source pin: `>&3` with fd 3 never opened fails the dup
    // at setup — `3: Bad file descriptor`, rc 1 (redir.c
    // redirection_error), unchanged by the write-face fix.
    let dir = tempdir("closed");
    seed(&dir, "f1", "cat-data\n");
    let run = run_script(&dir, "cat f1 >&3\necho rc=$?\n");
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "rc=1\n", "stderr: {}", run.stderr);
    assert!(
        run.stderr.contains("3: Bad file descriptor"),
        "stderr: {}",
        run.stderr
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue3980_echo_procsub_ro_fd_control_unchanged() {
    // Control: the true-builtin face already reported correctly —
    // `<(echo hi >&3)` on a read-only fd 3 emits
    // `echo: write error: Bad file descriptor` (builtins/common.c
    // sh_chkwrite via the routing walk's Closed model).
    let dir = tempdir("echoctl");
    let fd3src = seed(&dir, "fd3src", "seed3\n");
    let run = run_script(
        &dir,
        "exec 3<fd3src\ncat <(echo hi >&3)\nsleep 0.5\necho rc=$?\n",
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "rc=0\n", "stderr: {}", run.stderr);
    assert!(
        run.stderr
            .contains("echo: write error: Bad file descriptor"),
        "stderr: {}",
        run.stderr
    );
    assert_eq!(file_contents(&fd3src), "seed3\n");
    let _ = fs::remove_dir_all(&dir);
}
