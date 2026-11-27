//! rubash#3333 — simulated `mkdir -p` names the first failing
//! component in its diagnostic, like GNU coreutils mkdir.c
//! `make_ancestors`: the operand text truncated after the failing
//! component (`a/b` with `a` a file reports 'a'), not the whole
//! operand. The literal spelling survives (`a/./b`, `a//b`,
//! `./a`, `../x`). Single-level `mkdir` keeps reporting the
//! operand itself. Every expectation below is a GNU coreutils 9.4
//! (`/usr/bin/mkdir`) measurement, probed before the fix; the
//! rubash baseline printed the whole operand on all -p faces
//! (and answered rc 0 on `mkdir -p ""`, EEXIST on `mkdir ""`).
//!
//! Unix only: the deleted-cwd face rides the cwd anchor.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct ScratchDir(PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(case: &str) -> ScratchDir {
    let dir = std::env::temp_dir().join(format!("rubash-3333-{case}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch");
    let canonical = std::fs::canonicalize(&dir).expect("canonicalize scratch");
    ScratchDir(canonical)
}

fn run_rubash(dir: &Path, script: &str) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg("-c")
        .arg(script)
        .current_dir(dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        match child.try_wait().expect("poll rubash") {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                panic!("rubash did not exit within 30s");
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    use std::io::Read;
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    (stdout, stderr, status.code())
}

/// The deleted-cwd prelude: enter D/a, have a subshell remove it,
/// stay there.
fn prelude() -> &'static str {
    "cd a || exit 99\n(cd ..; rmdir a) || exit 98\n"
}

#[test]
fn issue3333_mkdir_p_names_first_component() {
    let dir = scratch("p-first");
    std::fs::write(dir.0.join("a"), "file\n").expect("write a");
    let (stdout, stderr, code) = run_rubash(&dir.0, "mkdir -p a/b; echo rc=$?\n");
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr:\n{stderr}");
    assert_eq!(
        stderr, "mkdir: cannot create directory 'a': Not a directory\n",
        "diagnostic must name the first failing component, not the operand"
    );
}

#[test]
fn issue3333_mkdir_p_names_prefix_through_second_component() {
    let dir = scratch("p-second");
    std::fs::create_dir_all(dir.0.join("a")).expect("create a");
    std::fs::write(dir.0.join("a/b"), "file\n").expect("write a/b");
    let (stdout, stderr, code) = run_rubash(&dir.0, "mkdir -p a/b/c; echo rc=$?\n");
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr:\n{stderr}");
    assert_eq!(
        stderr,
        "mkdir: cannot create directory 'a/b': Not a directory\n"
    );
}

#[test]
fn issue3333_mkdir_p_deleted_cwd_names_first_component() {
    let dir = scratch("p-dead");
    std::fs::create_dir_all(dir.0.join("a")).expect("create a");
    let script = format!("{}mkdir -p inner/deeper; echo rc=$?\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr:\n{stderr}");
    assert!(
        stderr.contains("mkdir: cannot create directory 'inner': No such file or directory\n"),
        "diagnostic must name 'inner', not the operand:\n{stderr}"
    );
    assert!(
        !dir.0.join("a").exists(),
        "the deleted directory must not be resurrected"
    );
}

#[test]
fn issue3333_mkdir_single_names_operand() {
    let dir = scratch("single");
    let (stdout, stderr, code) = run_rubash(&dir.0, "mkdir noexist/x; echo rc=$?\n");
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr:\n{stderr}");
    assert_eq!(
        stderr, "mkdir: cannot create directory 'noexist/x': No such file or directory\n",
        "single-level mkdir reports the operand itself"
    );
}

#[test]
fn issue3333_mkdir_p_empty_operand_errors() {
    let dir = scratch("p-empty");
    let (stdout, stderr, code) = run_rubash(&dir.0, "mkdir -p \"\"; echo rc=$?\n");
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr:\n{stderr}");
    assert_eq!(
        stderr, "mkdir: cannot create directory '': No such file or directory\n",
        "base answered rc 0 here; GNU errors ENOENT"
    );
}

#[test]
fn issue3333_mkdir_empty_operand_errors() {
    let dir = scratch("empty");
    let (stdout, stderr, code) = run_rubash(&dir.0, "mkdir \"\"; echo rc=$?\n");
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr:\n{stderr}");
    assert_eq!(
        stderr, "mkdir: cannot create directory '': No such file or directory\n",
        "base answered EEXIST here; GNU answers ENOENT"
    );
}

#[test]
fn issue3333_mkdir_p_dot_slash_prefix_kept() {
    let dir = scratch("p-dotslash");
    std::fs::write(dir.0.join("a"), "file\n").expect("write a");
    let (stdout, stderr, code) = run_rubash(&dir.0, "mkdir -p ./a/b; echo rc=$?\n");
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr:\n{stderr}");
    assert_eq!(
        stderr,
        "mkdir: cannot create directory './a': Not a directory\n"
    );
}

#[test]
fn issue3333_mkdir_p_dot_component_spelling_kept() {
    let dir = scratch("p-dotcomp");
    std::fs::create_dir_all(dir.0.join("a")).expect("create a");
    std::fs::write(dir.0.join("a/b"), "file\n").expect("write a/b");
    let (stdout, stderr, code) = run_rubash(&dir.0, "mkdir -p a/./b/c; echo rc=$?\n");
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr:\n{stderr}");
    assert_eq!(
        stderr,
        "mkdir: cannot create directory 'a/./b': Not a directory\n"
    );
}

#[test]
fn issue3333_mkdir_p_double_slash_spelling_kept() {
    let dir = scratch("p-dslash");
    std::fs::create_dir_all(dir.0.join("a")).expect("create a");
    std::fs::write(dir.0.join("a/b"), "file\n").expect("write a/b");
    let (stdout, stderr, code) = run_rubash(&dir.0, "mkdir -p a//b/c; echo rc=$?\n");
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr:\n{stderr}");
    assert_eq!(
        stderr,
        "mkdir: cannot create directory 'a//b': Not a directory\n"
    );
}

#[test]
fn issue3333_mkdir_p_absolute_names_prefix() {
    let dir = scratch("p-abs");
    std::fs::write(dir.0.join("f1"), "file\n").expect("write f1");
    let script = format!("mkdir -p {}/f1/x; echo rc=$?\n", dir.0.display());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr:\n{stderr}");
    assert_eq!(
        stderr,
        format!(
            "mkdir: cannot create directory '{}/f1': Not a directory\n",
            dir.0.display()
        )
    );
}

#[test]
fn issue3333_mkdir_p_parent_escape_prefix() {
    let dir = scratch("p-parent");
    std::fs::create_dir_all(dir.0.join("sub")).expect("create sub");
    std::fs::write(dir.0.join("x"), "file\n").expect("write x");
    let (stdout, stderr, code) = run_rubash(&dir.0, "cd sub; mkdir -p ../x/y; echo rc=$?\n");
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr:\n{stderr}");
    assert_eq!(
        stderr,
        "mkdir: cannot create directory '../x': Not a directory\n"
    );
}

#[test]
fn issue3333_mkdir_p_multi_operand_continues() {
    let dir = scratch("p-multi");
    std::fs::write(dir.0.join("f2"), "file\n").expect("write f2");
    let (stdout, stderr, code) = run_rubash(&dir.0, "mkdir -p ok1 f2/sub ok2; echo rc=$?\n");
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr:\n{stderr}");
    assert_eq!(
        stderr,
        "mkdir: cannot create directory 'f2': Not a directory\n"
    );
    assert!(dir.0.join("ok1").is_dir(), "ok1 must still be created");
    assert!(dir.0.join("ok2").is_dir(), "ok2 must still be created");
}

#[test]
fn issue3333_mkdir_p_dot_ok() {
    let dir = scratch("p-dot");
    let (stdout, stderr, code) = run_rubash(&dir.0, "mkdir -p .; echo rc=$?\n");
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr:\n{stderr}");
    assert_eq!(stderr, "", "stderr:\n{stderr}");
}

#[test]
fn issue3333_mkdir_p_success_chain() {
    let dir = scratch("p-ok");
    let (stdout, stderr, code) = run_rubash(&dir.0, "mkdir -p new/deep/deeper; echo rc=$?\n");
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr:\n{stderr}");
    assert_eq!(stderr, "", "stderr:\n{stderr}");
    assert!(
        dir.0.join("new/deep/deeper").is_dir(),
        "the whole chain must land"
    );
}

#[test]
fn issue3333_mkdir_p_symlink_to_dir_ok() {
    let dir = scratch("p-symdir");
    std::fs::create_dir_all(dir.0.join("real")).expect("create real");
    std::os::unix::fs::symlink("real", dir.0.join("a")).expect("symlink a");
    let (stdout, stderr, code) = run_rubash(&dir.0, "mkdir -p a/b/c; echo rc=$?\n");
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr:\n{stderr}");
    assert_eq!(stderr, "", "stderr:\n{stderr}");
    assert!(dir.0.join("real/b/c").is_dir(), "a/b/c must land in real");
}

#[test]
fn issue3333_mkdir_p_dangling_symlink_keeps_exist() {
    let dir = scratch("p-dangle");
    std::os::unix::fs::symlink("nowhere", dir.0.join("a")).expect("symlink a");
    let (stdout, stderr, code) = run_rubash(&dir.0, "mkdir -p a/b; echo rc=$?\n");
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr:\n{stderr}");
    assert_eq!(
        stderr, "mkdir: cannot create directory 'a': File exists\n",
        "GNU keeps the mkdir EEXIST when the follow-up stat fails"
    );
}

#[test]
fn issue3333_mkdir_p_dangling_symlink_single() {
    let dir = scratch("p-dangle1");
    std::os::unix::fs::symlink("nowhere", dir.0.join("a")).expect("symlink a");
    let (stdout, stderr, code) = run_rubash(&dir.0, "mkdir -p a; echo rc=$?\n");
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr:\n{stderr}");
    assert_eq!(stderr, "mkdir: cannot create directory 'a': File exists\n");
}

#[test]
fn issue3333_mkdir_p_root_ok() {
    let dir = scratch("p-root");
    let (stdout, stderr, code) = run_rubash(&dir.0, "mkdir -p /; echo rc=$?\n");
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr:\n{stderr}");
    assert_eq!(stderr, "", "stderr:\n{stderr}");
}
