//! rubash#3420: a background child spawned from a deleted cwd must sit
//! in the session cwd's inode — not fail with a `chdir:` diagnostic.
//!
//! GNU bash 5.3.20 oracle behavior: the session cwd IS the process cwd
//! (builtins/cd.def:600 `change_to_directory` moves it with `chdir`),
//! and the async child is a fork that inherits it —
//! execute_cmd.c:2811 `execute_connection` takes the `&` branch and
//! execute_cmd.c:5771 `execute_disk_command` forks via `make_child`
//! with no chdir in the child, so a deleted directory stays the
//! child's cwd and the job is silent with rc 0.
//!
//! rubash keeps the session cwd as a field and the raw-fork direct
//! spawn (`spawn_async_direct` -> `raw_fork_async_child`,
//! src/executor/external_inner.rs) seated the child with a path-string
//! `libc::chdir`, which fails once the directory is unlinked: the
//! diagnostic was printed to the shell's own stderr (before dup2, so
//! it bypassed the job's `2>` redirect) and the job exited 1. The fix
//! passes the cwd anchor fd (`session_cwd_anchor_fd`, the same
//! two-state ask `stamp_spawn_cwd` makes for `Command` spawns) into
//! the raw-fork child, which fchdirs the anchor first — the anchor
//! wins over a ghost path recreated at the same spelling — and only
//! falls back to the path spelling when there is no anchor.
//!
//! Expectations below are GNU 5.3.20 oracle measurements
//! (probe-3420.sh, saved at ~/workspace/fix-3420/probe-3420.sh):
//! every case passes there. Base behavior was measured on the
//! 685ccbc4 tree: the four divergence cells print
//! `bash: line N: /usr/bin/ls: chdir: <cwd>: No such file or
//! directory` on the shell's stderr with bg-rc=1 (C1a/C1b/C1c), and
//! the ghost cell lists `newmarker` (C1d).

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A scratch directory under the system temp dir, removed on drop.
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path =
            std::env::temp_dir().join(format!("rubash-issue3420-{tag}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Scratch { path }
    }

    fn mkdir(&self, name: &str) {
        std::fs::create_dir_all(self.path.join(name)).unwrap();
    }

    fn touch(&self, name: &str) {
        std::fs::write(self.path.join(name), b"").unwrap();
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn bg_ls_in_cd_deleted_cwd_is_silent() {
    let d = Scratch::new("ls-cd");
    d.mkdir("a");
    let dir = d.path.to_string_lossy();
    // cd entry: the deleted directory is empty; GNU's background `ls`
    // child sits in the deleted inode and prints nothing. Before the
    // fix the child died at the path-string chdir: the diagnostic
    // reached the shell's stderr and the job exited 1.
    let (stdout, stderr, code) = rubash(&format!(
        "cd '{dir}' || exit 1\nmkdir a\ncd a || exit 1\ntouch marker\n(cd ..; rm -rf a) || exit 1\nls >'{dir}/out' 2>'{dir}/err' & p=$!\nwait $p; echo \"bg-rc=$?\"\necho \"out=[$(cat '{dir}/out' 2>/dev/null)]\"\necho \"err=[$(cat '{dir}/err' 2>/dev/null)]\"\n"
    ));
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "bg-rc=0\nout=[]\nerr=[]\n");
}

#[test]
fn bg_ls_in_pushd_deleted_cwd_is_silent() {
    let d = Scratch::new("ls-pushd");
    d.mkdir("a");
    let dir = d.path.to_string_lossy();
    // pushd entry: same shape as the cd entry — the entry funnel does
    // not matter, the anchor is warmed by every cwd move.
    let (stdout, stderr, code) = rubash(&format!(
        "cd '{dir}' || exit 1\nmkdir a\npushd a >/dev/null || exit 1\ntouch marker\n(cd ..; rm -rf a) || exit 1\nls >'{dir}/out' 2>'{dir}/err' & p=$!\nwait $p; echo \"bg-rc=$?\"\necho \"out=[$(cat '{dir}/out' 2>/dev/null)]\"\necho \"err=[$(cat '{dir}/err' 2>/dev/null)]\"\n"
    ));
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "bg-rc=0\nout=[]\nerr=[]\n");
}

#[test]
fn bg_ls_relative_redirects_from_deleted_inode() {
    let d = Scratch::new("ls-relredir");
    d.mkdir("a");
    let dir = d.path.to_string_lossy();
    // The job's own redirects are relative (`../out`): they resolve
    // from the deleted inode, where `..` still names the scratch dir.
    // Before the fix the child died at the chdir before running `ls`.
    let (stdout, stderr, code) = rubash(&format!(
        "cd '{dir}' || exit 1\nmkdir a\ncd a || exit 1\ntouch marker\n(cd ..; rm -rf a) || exit 1\nls >../out 2>../err & p=$!\nwait $p; echo \"bg-rc=$?\"\necho \"out=[$(cat '{dir}/out' 2>/dev/null)]\"\necho \"err=[$(cat '{dir}/err' 2>/dev/null)]\"\n"
    ));
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "bg-rc=0\nout=[]\nerr=[]\n");
}

#[test]
fn bg_ls_ghost_lands_in_old_inode() {
    let d = Scratch::new("ls-ghost");
    d.mkdir("a");
    let dir = d.path.to_string_lossy();
    // Ghost: the directory is deleted AND recreated at the same path
    // (with a marker in the new one) before the spawn. GNU's child
    // sits in the OLD inode and lists nothing; a path-string chdir
    // lands in the new inode and lists `newmarker`.
    let (stdout, stderr, code) = rubash(&format!(
        "cd '{dir}' || exit 1\nmkdir a\ncd a || exit 1\ntouch marker\n(cd ..; rm -rf a; mkdir a; touch a/newmarker) || exit 1\nls >'{dir}/out' 2>'{dir}/err' & p=$!\nwait $p; echo \"bg-rc=$?\"\necho \"out=[$(cat '{dir}/out' 2>/dev/null)]\"\necho \"err=[$(cat '{dir}/err' 2>/dev/null)]\"\n"
    ));
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "bg-rc=0\nout=[]\nerr=[]\n");
}

#[test]
fn bg_ls_in_alive_cwd_lists_marker() {
    let d = Scratch::new("ls-alive");
    d.mkdir("a");
    let dir = d.path.to_string_lossy();
    // Control: with the cwd alive the anchor and the spelling name the
    // same inode, so the background `ls` lists the marker on every
    // tree (base included).
    let (stdout, stderr, code) = rubash(&format!(
        "cd '{dir}' || exit 1\nmkdir a\ncd a || exit 1\ntouch marker\nls >'{dir}/out' 2>'{dir}/err' & p=$!\nwait $p; echo \"bg-rc=$?\"\necho \"out=[$(cat '{dir}/out' 2>/dev/null)]\"\necho \"err=[$(cat '{dir}/err' 2>/dev/null)]\"\n"
    ));
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "bg-rc=0\nout=[marker]\nerr=[]\n");
}

#[test]
fn bg_false_in_deleted_cwd_keeps_command_rc() {
    let d = Scratch::new("false-deleted");
    d.mkdir("a");
    let dir = d.path.to_string_lossy();
    // Control: a background builtin never takes the direct external
    // spawn path, so its own rc 1 is reported with no chdir
    // diagnostic on every tree (base included).
    let (stdout, stderr, code) = rubash(&format!(
        "cd '{dir}' || exit 1\nmkdir a\ncd a || exit 1\n(cd ..; rm -rf a) || exit 1\nfalse >'{dir}/out' 2>'{dir}/err' & p=$!\nwait $p; echo \"bg-rc=$?\"\necho \"out=[$(cat '{dir}/out' 2>/dev/null)]\"\necho \"err=[$(cat '{dir}/err' 2>/dev/null)]\"\n"
    ));
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "bg-rc=1\nout=[]\nerr=[]\n");
}

#[test]
fn bg_missing_command_in_deleted_cwd_keeps_127() {
    let d = Scratch::new("missing-deleted");
    d.mkdir("a");
    let dir = d.path.to_string_lossy();
    // Control: a background job naming a missing command still reports
    // 127 with the `command not found` diagnostic and no `chdir:`
    // line on every tree (base included) — the fix must not swallow
    // real spawn failures while seating the child.
    let (stdout, stderr, code) = rubash(&format!(
        "cd '{dir}' || exit 1\nmkdir a\ncd a || exit 1\n(cd ..; rm -rf a) || exit 1\nno-such-cmd-xyz >'{dir}/out' 2>'{dir}/err' & p=$!\nwait $p; echo \"bg-rc=$?\"\necho \"out=[$(cat '{dir}/out' 2>/dev/null)]\"\necho \"err=[$(cat '{dir}/err' 2>/dev/null)]\"\n"
    ));
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
    assert!(
        stdout.contains("bg-rc=127"),
        "job rc should be 127, got: {stdout}"
    );
    assert!(
        stdout.contains("command not found"),
        "missing command should be diagnosed, got: {stdout}"
    );
    assert!(
        !stdout.contains("chdir"),
        "no chdir diagnostic should appear, got: {stdout}"
    );
}
