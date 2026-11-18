//! rubash#3268 face (d) — the in-process emulated `cp` operand face
//! from a deleted session cwd.
//!
//! GNU bash has no cp builtin: bash forks a child (execute_cmd.c
//! execute_disk_command) whose cwd IS the deleted directory inode,
//! and coreutils cp's relative operands are walked by the kernel
//! from that inode — `../f` reaches the real parent, while a create
//! aimed INTO the dead directory itself fails ENOENT. Under G-02
//! rubash's emulated cp ran in the shell process and resolved every
//! operand through the session-cwd STRING (`<cwd>/../f`), which dies
//! at the deleted component — the source stat failed before any
//! kernel walk could start.
//!
//! The fix converges each operand ROOT once, at the external_cp
//! entry: a relative operand whose session cwd no longer resolves
//! (the rubash#3275 identity verdict — a path recreated around the
//! stationary session counts as dead) with an anchor naming that cwd
//! becomes an anchor-relative root, and the single copy engine walks
//! nodes under it with the *at family, so the kernel performs the
//! same walk GNU's child gets. Anything else keeps the plain joined
//! path, byte for byte. Textual normalization of the operand was
//! measured and rejected (design §6.2): it dies on the root-itself
//! cell, crosses into recreated ghost directories, and needs /proc.
//!
//! Oracle: GNU bash 5.3.20 (probe-3268-d.sh on the dispatch tree),
//! deleted cwd staged by `cd a; (cd ..; rmdir a)`:
//!   cp ../f ../g2            -> rc 0, g2 = hello
//!   cp -r ../sub ../subcopy  -> rc 0, subcopy/sf = world
//!   cp -r ../sub ../sc2      -> rc 0, sc2/deep/df = deep
//!   cp -r . ../selfcopy      -> rc 0, selfcopy an empty directory
//!   cp ../f g                -> rc 1, CREATE side (cannot create
//!                               regular file 'g'), no g, source whole
//!   cp -r ../sub sc          -> rc 1, CREATE side, no sc
//!   cp $PWD/../f ../g3       -> rc 1, STAT side (absolute dead
//!                               spelling stays dead)
//!   cp -r ../sub/. ../dotcopy-> rc 0, children land directly
//!   ghost (a recreated in place by an external process):
//!     cp ../f ../g2          -> rc 0 (old world), cp marker ../gm
//!                               -> rc 1 cannot stat, no gm
//!
//! These pins assert COMPLETION FORMS (output bytes + exit status +
//! physical landings), never timing. The create-side cells assert
//! the failure SIDE (stderr family), not verbatim diagnostic text:
//! the emulation's create diagnostic renders the joined target path
//! where coreutils renders the operand word — a pre-existing wording
//! delta outside this face. Unix only: the cwd anchor is a Unix
//! directory handle; off Unix there is no anchor and the face keeps
//! its pre-fix shape.

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

/// A canonicalized scratch root D: D/f = hello, D/sub/sf = world,
/// D/sub/deep/df = deep, D/a the directory the shell enters and
/// loses. Canonicalized because later assertions compare physical
/// paths under D.
fn scratch(case: &str) -> ScratchDir {
    let dir = std::env::temp_dir().join(format!("rubash-3268d-{case}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("a")).expect("create a");
    std::fs::create_dir_all(dir.join("sub/deep")).expect("create sub/deep");
    std::fs::write(dir.join("f"), "hello\n").expect("write f");
    std::fs::write(dir.join("sub/sf"), "world\n").expect("write sf");
    std::fs::write(dir.join("sub/deep/df"), "deep\n").expect("write df");
    let canonical = std::fs::canonicalize(&dir).expect("canonicalize scratch");
    ScratchDir(canonical)
}

/// Spawn rubash on `script` inside `dir`; returns (stdout, stderr,
/// status code).
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

/// The deleted-cwd prelude shared by the dead-session pins: enter
/// D/a, have a subshell remove it, stay there.
fn prelude() -> &'static str {
    "cd a || exit 99\n(cd ..; rmdir a) || exit 98\n"
}

fn landed(dir: &Path, rel: &str) -> String {
    std::fs::read_to_string(dir.join(rel))
        .unwrap_or_else(|error| panic!("expected {rel} to land under {}: {error}", dir.display()))
}

// ---------------------------------------------------------------- fail-first

#[test]
fn issue3268_cp_updest() {
    let dir = scratch("d1");
    let script = format!("{}cp ../f ../g2; echo rc=$?\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr:\n{stderr}");
    assert_eq!(landed(&dir.0, "g2"), "hello\n");
}

#[test]
fn issue3268_cp_tree_updest() {
    let dir = scratch("d2");
    let script = format!("{}cp -r ../sub ../subcopy; echo rc=$?\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr:\n{stderr}");
    assert_eq!(landed(&dir.0, "subcopy/sf"), "world\n");
}

#[test]
fn issue3268_cp_tree_deep() {
    // Two levels below the anchored root: proves the engine walks
    // node-by-node under the anchor, not a root-only patch.
    let dir = scratch("d3");
    let script = format!("{}cp -r ../sub ../sc2; echo rc=$?\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr:\n{stderr}");
    assert_eq!(landed(&dir.0, "sc2/sf"), "world\n");
    assert_eq!(landed(&dir.0, "sc2/deep/df"), "deep\n");
}

#[test]
fn issue3268_cp_dot_root() {
    // The source root is the deleted directory inode itself — the
    // cell that kills any textual-normalization scheme (its joined
    // spelling `<cwd>/..` + text surgery can never name the inode).
    let dir = scratch("d4");
    let script = format!("{}cp -r . ../selfcopy; echo rc=$?\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr:\n{stderr}");
    let selfcopy = dir.0.join("selfcopy");
    assert!(selfcopy.is_dir(), "selfcopy must be a directory");
    let entries: Vec<_> = std::fs::read_dir(&selfcopy)
        .expect("read selfcopy")
        .collect();
    assert!(entries.is_empty(), "selfcopy must be empty: {entries:?}");
}

#[test]
fn issue3268_cp_dest_in_dead_fails_at_create() {
    // GNU also fails this shape — but on the CREATE side (the source
    // reads fine; the kernel refuses to create inside the dead
    // directory). Base rubash fails on the source STAT instead, so
    // the rc alone cannot carry the cell: the failure side is the
    // assertion (stderr family, not verbatim text).
    let dir = scratch("d5");
    let script = format!("{}cp ../f g; echo rc=$?\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr:\n{stderr}");
    assert!(
        stderr.contains("cannot create"),
        "failure must be on the create side:\n{stderr}"
    );
    assert!(
        !stderr.contains("cannot stat"),
        "source stat must have succeeded:\n{stderr}"
    );
    assert!(!dir.0.join("g").exists(), "g must not be created");
    assert_eq!(landed(&dir.0, "f"), "hello\n", "source must be whole");
}

#[test]
fn issue3268_cp_tree_dest_in_dead_fails_at_create() {
    let dir = scratch("d6");
    let script = format!("{}cp -r ../sub sc; echo rc=$?\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr:\n{stderr}");
    assert!(
        stderr.contains("cannot create"),
        "failure must be on the create side:\n{stderr}"
    );
    assert!(
        !stderr.contains("cannot stat"),
        "source stat must have succeeded:\n{stderr}"
    );
    assert!(!dir.0.join("sc").exists(), "sc must not be created");
}

#[test]
fn issue3268_cp_source_dot_contents() {
    // The `src/.` contents form: its enumeration root must converge
    // onto the same anchor rule (design §6.1's contents touchpoint).
    let dir = scratch("d9");
    let script = format!("{}cp -r ../sub/. ../dotcopy; echo rc=$?\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr:\n{stderr}");
    assert_eq!(landed(&dir.0, "dotcopy/sf"), "world\n");
    assert_eq!(landed(&dir.0, "dotcopy/deep/df"), "deep\n");
    assert!(
        !dir.0.join("dotcopy/sub").exists(),
        "contents form must not nest a sub level"
    );
}

#[test]
fn issue3268_cp_ghost_not_crossed() {
    // The rubash#3275 identity verdict applied to cp (captain ruling:
    // the ghost semantics landed BEFORE this face, so this cell is
    // written at GNU values as fail-first): `a` is deleted, then an
    // external process recreates it in place with a marker that
    // exists ONLY in the ghost. The stationary session still sits in
    // the dead inode: the parent's f copies out, the ghost's marker
    // does not exist for it.
    let dir = scratch("d10");
    let recreate = format!(
        "/bin/sh -c 'mkdir {}/a && printf \"ghost\\n\" > {}/a/marker' || exit 97\n",
        dir.0.display(),
        dir.0.display()
    );
    let script = format!(
        "{}{}cp ../f ../g2; echo rc=$?; cp marker ../gm; echo grc=$?\n",
        prelude(),
        recreate
    );
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=0\ngrc=1\n", "stderr:\n{stderr}");
    assert!(
        stderr.contains("cannot stat") && stderr.contains("marker"),
        "the ghost's marker must not stat for the dead session:\n{stderr}"
    );
    assert_eq!(landed(&dir.0, "g2"), "hello\n", "old world must copy");
    assert!(!dir.0.join("gm").exists(), "gm must not be created");
    // The ghost itself is untouched by the session's copies.
    assert_eq!(landed(&dir.0, "a/marker"), "ghost\n");
}

// ---------------------------------------------------------------- regression locks

#[test]
fn issue3268_cp_abs_dead_source_stays_dead() {
    // Control (both shells, oracle-measured): an absolute spelling
    // that textually traverses the deleted directory dies on the
    // stat side — the anchor route is for relative operands only.
    let dir = scratch("d7");
    let script = format!("{}cp $PWD/../f ../g3; echo rc=$?\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr:\n{stderr}");
    assert!(
        stderr.contains("cannot stat"),
        "absolute dead spelling must fail at stat:\n{stderr}"
    );
    assert!(!dir.0.join("g3").exists(), "g3 must not be created");
}

#[test]
fn issue3268_cp_live_cwd_suite() {
    // Live-cwd guard: the root convergence must leave the ordinary
    // shapes byte-identical (plain roots). The full cp pin family
    // (issue1941 et al.) carries the width; this cell keeps one
    // in-stem witness, including a --preserve=links pair.
    use std::os::unix::fs::MetadataExt;
    let dir = scratch("d8");
    std::fs::write(dir.0.join("h1"), "linked\n").expect("write h1");
    std::fs::hard_link(dir.0.join("h1"), dir.0.join("h2")).expect("hard link h2");
    let script = "\
cp f g2; echo rc1=$?
cp -r sub scopy; echo rc2=$?
cp f sub; echo rc3=$?
cp f sub/sf scopy; echo rc4=$?
cp --preserve=links h1 h2 scopy; echo rc5=$?
";
    let (stdout, stderr, code) = run_rubash(&dir.0, script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(
        stdout, "rc1=0\nrc2=0\nrc3=0\nrc4=0\nrc5=0\n",
        "stderr:\n{stderr}"
    );
    assert_eq!(landed(&dir.0, "g2"), "hello\n");
    assert_eq!(landed(&dir.0, "scopy/sf"), "world\n");
    assert_eq!(landed(&dir.0, "scopy/deep/df"), "deep\n");
    assert_eq!(landed(&dir.0, "sub/f"), "hello\n");
    assert_eq!(landed(&dir.0, "scopy/f"), "hello\n");
    let h1 = std::fs::metadata(dir.0.join("scopy/h1")).expect("scopy/h1");
    let h2 = std::fs::metadata(dir.0.join("scopy/h2")).expect("scopy/h2");
    assert_eq!(
        (h1.dev(), h1.ino()),
        (h2.dev(), h2.ino()),
        "--preserve=links must keep the pair linked"
    );
}
