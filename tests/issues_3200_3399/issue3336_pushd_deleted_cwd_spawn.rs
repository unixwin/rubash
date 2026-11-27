//! rubash#3336: after `pushd` into a directory that is then deleted,
//! external children must inherit the deleted cwd — not the shell's
//! startup cwd.
//!
//! GNU bash 5.3.20 oracle behavior (builtins/pushd.def:298 pushd Dir
//! delegates to cd_builtin; pushd.def:265/:284 rotations and :400 popd
//! go through change_to_temp/cd_to_string -> cd_builtin;
//! builtins/cd.def:600 change_to_directory moves the PROCESS cwd with
//! chdir; execute_cmd.c:5771 execute_disk_command forks via make_child
//! with no chdir in the child, so the child inherits the shell's cwd
//! — a deleted directory stays the child's cwd, and its getcwd fails).
//!
//! rubash keeps the session cwd as a field (G-02: the process never
//! chdirs) and stamps spawns from a cwd-anchor fd
//! (stamp_spawn_cwd). `cd` warms that anchor on every move, but
//! pushd/popd never did, so after a pushd-entered directory was
//! deleted the stamp fell through unstamped and the child inherited
//! the process (startup) cwd.
//!
//! Expectations below are GNU 5.3.20 oracle measurements
//! (/tmp/gnu-bash-3349/bash, 2026-11-11): every case passes there.

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
            std::env::temp_dir().join(format!("rubash-issue3336-{tag}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Scratch { path }
    }

    fn mkdir(&self, name: &str) {
        std::fs::create_dir_all(self.path.join(name)).unwrap();
    }

    fn touch(&self, name: &str) {
        std::fs::write(self.path.join(name), b"").unwrap();
    }

    fn write(&self, name: &str, contents: &[u8]) {
        std::fs::write(self.path.join(name), contents).unwrap();
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
fn ls_in_pushd_deleted_cwd_is_empty() {
    let d = Scratch::new("ls-pushd");
    d.mkdir("a");
    d.touch("marker");
    let dir = d.path.to_string_lossy();
    // The deleted directory is empty; GNU's `ls` child inherits the
    // deleted cwd and prints nothing. Before the fix the child ran in
    // the startup cwd and listed `marker` (and the repo tree).
    let (stdout, _, code) = rubash(&format!(
        "cd '{dir}' || exit 1\npushd a >/dev/null || exit 1\n(cd ..; rmdir a) || exit 1\nx=$(ls 2>&1)\nif [ -n \"$x\" ]; then echo \"GOT [$x]\"; else echo EMPTY; fi\n"
    ));
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "EMPTY\n");
}

#[test]
fn sh_child_in_pushd_deleted_cwd_reports_getcwd_failure() {
    let d = Scratch::new("sh-pushd");
    d.mkdir("a");
    let dir = d.path.to_string_lossy();
    // GNU's `sh -c` child inherits the deleted cwd: dash reports the
    // getcwd failure and still runs the command. Before the fix the
    // child had a valid (startup) cwd and reported nothing.
    let (stdout, _, code) = rubash(&format!(
        "cd '{dir}' || exit 1\npushd a >/dev/null || exit 1\n(cd ..; rmdir a) || exit 1\ny=$(sh -c 'echo inner' 2>&1)\necho \"$y\"\n"
    ));
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("getcwd"),
        "sh child should report a getcwd failure, got: {stdout}"
    );
    assert!(
        stdout.contains("inner"),
        "sh child should run, got: {stdout}"
    );
}

#[test]
fn ls_in_popd_deleted_cwd_is_empty() {
    let d = Scratch::new("ls-popd");
    d.mkdir("a");
    d.mkdir("b");
    d.touch("b/bmarker");
    let dir = d.path.to_string_lossy();
    // popd's top-of-stack move (pushd.def:400 cd_to_string) is a cwd
    // mover too: after `popd` lands in a, deleting a must leave the
    // `ls` child in the deleted cwd. The bmarker pins the landing:
    // a wrong landing in b would list it.
    let (stdout, _, code) = rubash(&format!(
        "cd '{dir}' || exit 1\npushd a >/dev/null || exit 1\npushd ../b >/dev/null || exit 1\npopd >/dev/null || exit 1\n(cd ..; rmdir a) || exit 1\nx=$(ls 2>&1)\nif [ -n \"$x\" ]; then echo \"GOT [$x]\"; else echo EMPTY; fi\n"
    ));
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "EMPTY\n");
}

#[test]
fn ls_in_pushd_rotated_deleted_cwd_is_empty() {
    let d = Scratch::new("ls-rotate");
    d.mkdir("a");
    d.mkdir("b");
    d.touch("b/bmarker");
    let dir = d.path.to_string_lossy();
    // `pushd +1` rotates the stack (pushd.def:265 change_to_temp):
    // the rotation's chdir lands in a, so deleting a must leave the
    // `ls` child in the deleted cwd.
    let (stdout, _, code) = rubash(&format!(
        "cd '{dir}' || exit 1\npushd a >/dev/null || exit 1\npushd ../b >/dev/null || exit 1\npushd +1 >/dev/null || exit 1\n(cd ..; rmdir a) || exit 1\nx=$(ls 2>&1)\nif [ -n \"$x\" ]; then echo \"GOT [$x]\"; else echo EMPTY; fi\n"
    ));
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "EMPTY\n");
}

#[test]
fn ls_in_pushd_swapped_deleted_cwd_is_empty() {
    let d = Scratch::new("ls-swap");
    d.mkdir("a");
    d.mkdir("b");
    d.touch("b/bmarker");
    let dir = d.path.to_string_lossy();
    // Bare `pushd` swaps the top two entries (pushd.def:209
    // change_to_temp): the swap's chdir lands back in a, so deleting
    // a must leave the `ls` child in the deleted cwd.
    let (stdout, _, code) = rubash(&format!(
        "cd '{dir}' || exit 1\npushd a >/dev/null || exit 1\npushd ../b >/dev/null || exit 1\npushd >/dev/null || exit 1\n(cd ..; rmdir a) || exit 1\nx=$(ls 2>&1)\nif [ -n \"$x\" ]; then echo \"GOT [$x]\"; else echo EMPTY; fi\n"
    ));
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "EMPTY\n");
}

#[test]
fn pipeline_ls_in_pushd_deleted_cwd_is_empty() {
    let d = Scratch::new("ls-pipe");
    d.mkdir("a");
    d.touch("marker");
    let dir = d.path.to_string_lossy();
    // Pipeline stages go through the same spawn stamp, so `ls | cat`
    // in the deleted pushd cwd must also see the empty deleted
    // directory, not the startup cwd.
    let (stdout, _, code) = rubash(&format!(
        "cd '{dir}' || exit 1\npushd a >/dev/null || exit 1\n(cd ..; rmdir a) || exit 1\nx=$(ls 2>&1 | cat)\nif [ -n \"$x\" ]; then echo \"GOT [$x]\"; else echo EMPTY; fi\n"
    ));
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "EMPTY\n");
}

#[test]
fn cd_entry_deleted_cwd_controls_agree() {
    let d = Scratch::new("cd-control");
    d.mkdir("b2");
    d.write("f", b"hi\n");
    let dir = d.path.to_string_lossy();
    // Control: the cd-entry form already works (cd warms the anchor),
    // and absolute-path children still run in a deleted cwd.
    let (stdout, _, code) = rubash(&format!(
        "cd '{dir}' || exit 1\ncd b2 || exit 1\n(cd ..; rmdir b2) || exit 1\nz=$(ls 2>&1)\nif [ -n \"$z\" ]; then echo \"FAIL ls [$z]\"; else echo PASS-ls; fi\nid >/dev/null 2>&1; echo \"id rc=$?\"\nwc '{dir}/f' >/dev/null 2>&1; echo \"wc rc=$?\"\n"
    ));
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "PASS-ls\nid rc=0\nwc rc=0\n");
}
