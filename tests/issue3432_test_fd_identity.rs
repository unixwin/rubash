//! rubash#3432 — `test -ef`/`-nt`/`-ot` on a /proc/self/fd/N or
//! /dev/fd/N operand resolved fd N to the file open on fd N-1 (and
//! fd 3 to the proc-entry metadata, mtime "now").
//!
//! Root cause: the shell's OS fd namespace is shifted — the process
//! permanently holds its cwd anchor at OS fd 3, so user fd 3's file
//! lands on the next free OS fd and every higher user fd cascades
//! (measured: `exec 3< f; exec 4< g` holds f on OS fd 4, g on OS
//! fd 5). The binary file operators stat'ed the literal path, so
//! /proc/self/fd/N answered for OS fd N — the previous user fd's
//! file. The unary faces already answered from the fd-table marks;
//! the binary faces never consulted them.
//!
//! The fix mints a `__RUBASH_FD_STAT_<fd>` mark per entry (fstat(2)
//! on the entry's kernel handle: dev, inode, mtime sec/nsec) beside
//! the TARGET/ACCESS facts, and the binary operators resolve an
//! fd-alias operand through the marks (STAT, then TARGET path, then
//! the literal path). The `[[ ]]` binary arm never synced the marks
//! at all — it does now, mirroring the unary arm.
//!
//! Oracle: GNU bash 5.3.20 (hunt seeds seed_r108a.sh, 10/10, and
//! seed_r108e.sh, 17/17, in
//! ~/workspace/rubash-hunt-runs/2026-11-12-r108/; verdict on master
//! 6c09b5c4: r108a 6 FAIL + 4 PASS, r108e 10 FAIL + 7 PASS). GNU
//! semantics: test.c:335-352 filecomp — `-ef` is same_file
//! (general.c:673: plain stat on both paths, compare (st_dev,
//! st_ino)); `-nt`/`-ot` compare stat mtimes; sh_stat
//! (lib/sh/eaccess.c:80) stats the descriptor the process holds,
//! and in GNU the shell's OS fd N IS user fd N.
//!
//! Unix only: /proc/self/fd is a unix face. Every cell asserts an
//! exit status only — no timing, no output bytes. Mtimes are set
//! from Rust (2020 vs 2021) so no cell depends on the emulated
//! `touch -d`.

#![cfg(unix)]

use std::fs::FileTimes;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

struct ScratchDir(PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// f (mtime 2020-01-01) and g (mtime 2021-01-01) in a fresh scratch dir.
fn scratch(case: &str) -> ScratchDir {
    let dir = std::env::temp_dir().join(format!("rubash-3432-{case}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch");
    for (name, secs) in [("f", 1_577_836_800u64), ("g", 1_609_459_200u64)] {
        let path = dir.join(name);
        std::fs::write(&path, format!("{name}\n")).expect("write file");
        let file = std::fs::File::open(&path).expect("open file");
        file.set_times(
            FileTimes::new().set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs)),
        )
        .expect("set mtime");
    }
    let canonical = std::fs::canonicalize(&dir).expect("canonicalize scratch");
    ScratchDir(canonical)
}

/// Spawn rubash on `script` inside `dir`; returns the exit status.
fn run_rubash(dir: &Path, script: &str) -> Option<i32> {
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
            None => {
                assert!(Instant::now() < deadline, "rubash timed out: {script}");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    };
    let output = child.wait_with_output().expect("collect rubash output");
    assert_eq!(output.status.code(), status.code());
    status.code()
}

fn assert_rc(dir: &Path, script: &str, want_rc: i32) {
    let rc = run_rubash(dir, script);
    assert_eq!(rc, Some(want_rc), "rc for {script:?}");
}

/// `exec 3< f; exec 4< g` — fd 3 on the 2020 file, fd 4 on the 2021 file.
const SETUP: &str = "exec 3< f; exec 4< g; ";

#[test]
fn ef_fd_operands_resolve_to_the_bound_file() {
    // seed_r108a cells 1-6: every -ef shape that was false at base.
    let dir = scratch("ef");
    assert_rc(&dir.0, &format!("{SETUP}test f -ef /proc/self/fd/3"), 0);
    assert_rc(&dir.0, &format!("{SETUP}test /proc/self/fd/3 -ef f"), 0);
    assert_rc(
        &dir.0,
        &format!("{SETUP}exec 4< f; test /proc/self/fd/3 -ef /proc/self/fd/4"),
        0,
    );
    assert_rc(&dir.0, &format!("{SETUP}test f -ef /dev/fd/3"), 0);
    assert_rc(
        &dir.0,
        &format!("{SETUP}test \"$PWD/f\" -ef /proc/self/fd/3"),
        0,
    );
    assert_rc(
        &dir.0,
        &format!("{SETUP}ln -s f s; exec 3< s; test f -ef /proc/self/fd/3"),
        0,
    );
    // The shift's smoking gun, both directions: fd 4 is g, not f.
    assert_rc(&dir.0, &format!("{SETUP}test /proc/self/fd/4 -ef g"), 0);
    assert_rc(&dir.0, &format!("{SETUP}test /proc/self/fd/4 -ef f"), 1);
    assert_rc(&dir.0, &format!("{SETUP}test g -ef /proc/self/fd/4"), 0);
    assert_rc(&dir.0, &format!("{SETUP}test f -ef /dev/fd/4"), 1);
    assert_rc(&dir.0, &format!("{SETUP}test g -ef /dev/fd/4"), 0);
}

#[test]
fn ef_fd_controls_still_agree() {
    // seed_r108a cells 7-10: identical fd paths, normal symlink,
    // missing operand, fd on a directory — all green at base, locks.
    let dir = scratch("ef-ctl");
    assert_rc(
        &dir.0,
        &format!("{SETUP}test /proc/self/fd/3 -ef /proc/self/fd/3"),
        0,
    );
    assert_rc(&dir.0, "ln -s f s; test s -ef f", 0);
    assert_rc(&dir.0, "test f -ef missing", 1);
    assert_rc(&dir.0, "exec 3< .; test . -ef /proc/self/fd/3", 0);
    assert_rc(&dir.0, "exec 3< .; test /proc/self/fd/3 -ef .", 0);
    // Distinct files stay distinct through their fds.
    assert_rc(
        &dir.0,
        &format!("{SETUP}test /proc/self/fd/3 -ef /proc/self/fd/4"),
        1,
    );
    assert_rc(&dir.0, &format!("{SETUP}test /proc/self/fd/3 -ef g"), 1);
}

#[test]
fn nt_ot_fd_operands_compare_the_bound_files_mtime() {
    // seed_r108e cells 1-14: f is 2020, g is 2021; fd 3 is f, fd 4 is g.
    let dir = scratch("nt-ot");
    assert_rc(&dir.0, &format!("{SETUP}test /proc/self/fd/3 -nt g"), 1);
    assert_rc(&dir.0, &format!("{SETUP}test g -nt /proc/self/fd/3"), 0);
    assert_rc(&dir.0, &format!("{SETUP}test /proc/self/fd/3 -nt f"), 1);
    assert_rc(&dir.0, &format!("{SETUP}test /proc/self/fd/4 -nt f"), 0);
    assert_rc(&dir.0, &format!("{SETUP}test f -nt /proc/self/fd/4"), 1);
    assert_rc(&dir.0, &format!("{SETUP}test /proc/self/fd/4 -nt g"), 1);
    assert_rc(&dir.0, &format!("{SETUP}test /proc/self/fd/3 -ot g"), 0);
    assert_rc(&dir.0, &format!("{SETUP}test g -ot /proc/self/fd/3"), 1);
    assert_rc(&dir.0, &format!("{SETUP}test /proc/self/fd/3 -ot f"), 1);
    assert_rc(&dir.0, &format!("{SETUP}test f -ot /proc/self/fd/3"), 1);
    assert_rc(&dir.0, &format!("{SETUP}test /proc/self/fd/4 -ot f"), 1);
    assert_rc(&dir.0, &format!("{SETUP}test f -ot /proc/self/fd/4"), 0);
    assert_rc(&dir.0, &format!("{SETUP}test /dev/fd/4 -nt f"), 0);
    assert_rc(&dir.0, &format!("{SETUP}test /dev/fd/3 -nt g"), 1);
    assert_rc(&dir.0, &format!("{SETUP}test /dev/fd/3 -ot g"), 0);
    assert_rc(&dir.0, &format!("{SETUP}test /dev/fd/4 -ot f"), 1);
}

#[test]
fn nt_ot_controls_still_agree() {
    // seed_r108e cells 15-17: plain files and the identical-fd-path
    // -ef control — green at base, locks.
    let dir = scratch("nt-ot-ctl");
    assert_rc(&dir.0, &format!("{SETUP}test f -nt g"), 1);
    assert_rc(&dir.0, &format!("{SETUP}test g -nt f"), 0);
    assert_rc(&dir.0, &format!("{SETUP}test f -ot g"), 0);
    assert_rc(&dir.0, &format!("{SETUP}test g -ot f"), 1);
    assert_rc(
        &dir.0,
        &format!("{SETUP}test /proc/self/fd/3 -ef /proc/self/fd/3"),
        0,
    );
}

#[test]
fn bracket_forms_share_the_fix() {
    // [ ] syncs through the dispatch scan; [[ ]] binary ops never
    // synced the marks at all before this fix (conditional.rs) —
    // both forms must answer from the bound file, not the shifted
    // OS fd.
    let dir = scratch("brackets");
    for (form, close) in [("test", ""), ("[", " ]"), ("[[", " ]]")] {
        let run = |dir: &Path, expr: &str| {
            let script = if form == "test" {
                format!("{SETUP}test {expr}")
            } else {
                format!("{SETUP}{form} {expr}{close}")
            };
            run_rubash(dir, &script)
        };
        assert_eq!(
            run(&dir.0, "f -ef /proc/self/fd/3"),
            Some(0),
            "{form} -ef fd3==f"
        );
        assert_eq!(
            run(&dir.0, "/proc/self/fd/4 -ef g"),
            Some(0),
            "{form} -ef fd4==g"
        );
        assert_eq!(
            run(&dir.0, "/proc/self/fd/4 -ef f"),
            Some(1),
            "{form} -ef fd4!=f"
        );
        assert_eq!(
            run(&dir.0, "/proc/self/fd/4 -nt f"),
            Some(0),
            "{form} -nt fd4>g>f"
        );
        assert_eq!(
            run(&dir.0, "/proc/self/fd/3 -ot g"),
            Some(0),
            "{form} -ot fd3<f<g"
        );
        assert_eq!(run(&dir.0, "f -nt /dev/fd/4"), Some(1), "{form} -nt f<fd4");
    }
}

#[test]
fn dup_pipe_and_deleted_fd_faces() {
    // A dup shares the source's open file description, so its STAT
    // mark travels with it (the overlay's DupOf arm). A pipe fd and
    // a deleted file's fd have no re-stat-able path — only the STAT
    // mark names them.
    let dir = scratch("dup-pipe");
    assert_rc(
        &dir.0,
        &format!("{SETUP}exec 4<&3; test /proc/self/fd/4 -ef f"),
        0,
    );
    assert_rc(
        &dir.0,
        &format!("{SETUP}exec 4<&3; test /proc/self/fd/4 -ef g"),
        1,
    );
    assert_rc(
        &dir.0,
        &format!("{SETUP}exec 4<&3; test /proc/self/fd/4 -nt g"),
        1,
    );
    assert_rc(
        &dir.0,
        &format!("{SETUP}exec 4<&3; test g -nt /proc/self/fd/4"),
        0,
    );
    assert_rc(
        &dir.0,
        "exec 3< <(printf x); test /proc/self/fd/3 -ef /proc/self/fd/3",
        0,
    );
    assert_rc(
        &dir.0,
        "exec 3< <(printf x); exec 4<&3; test /proc/self/fd/3 -ef /proc/self/fd/4",
        0,
    );
    assert_rc(
        &dir.0,
        "exec 3< f; rm f; test /proc/self/fd/3 -ef /proc/self/fd/3",
        0,
    );
    assert_rc(&dir.0, "exec 3< f; rm f; test /proc/self/fd/3 -ef g", 1);
}
