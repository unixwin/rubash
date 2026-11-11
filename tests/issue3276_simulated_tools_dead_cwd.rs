//! rubash#3276 — the in-process emulated `mkdir` / `rm` / `touch`
//! operand face from a deleted session cwd (the #3268 family census
//! found all three red on the same mechanism as face (d)'s cp).
//!
//! GNU bash has no mkdir/rm/touch builtins: bash forks a child
//! (execute_cmd.c execute_disk_command) whose cwd IS the deleted
//! directory inode, and the coreutils tool's relative operands are
//! walked by the kernel from that inode — `../f` reaches the real
//! parent, while a create/delete aimed INTO the dead directory
//! itself fails ENOENT. Under G-02 rubash's emulated tools ran in
//! the shell process and resolved every operand through the
//! session-cwd STRING (`<cwd>/../f`), which dies at the deleted
//! component — and, once the path was recreated in place, silently
//! crossed into the recreated GHOST directory (base measurements:
//! `rm marker` deleted the ghost's marker rc 0; `mkdir -p` even
//! RESURRECTED the deleted directory and built inside it).
//!
//! The fix converges each operand root ONCE at the handler entry,
//! through the very convergence face (d) built for cp
//! (Executor::cp_operand_root over the rubash#3275 identity
//! verdict): a relative operand under a deleted session cwd becomes
//! an anchor-relative CpNode, and mkdir/rm/touch operate on it with
//! the *at family (mkdirat / unlinkat / utimensat against the
//! anchor handle), so the kernel performs the walk GNU's forked
//! child gets. There is no second gate: the guards (rm's `.`/`..`
//! refusal and preserve-root) read the same node's stat. Anything
//! else — absolute operand, live cwd, no anchor — keeps the plain
//! joined path, byte for byte.
//!
//! Oracle: GNU bash 5.3.20 + system coreutils (probe-3276.sh on the
//! dispatch tree; base rubash measured on master 5d8867fd),
//! deleted cwd staged by `cd a; (cd ..; rmdir a)`:
//!   mkdir ../newdir        -> rc 0 (base rc 1)
//!   mkdir -p ../pdir/deep  -> rc 0, a NOT resurrected (base rc 0
//!                             but resurrected a and built there)
//!   mkdir -p inner/deeper  -> rc 1 create side (base rc 0, built
//!                             inside the resurrected directory)
//!   mkdir newdir           -> rc 1 ENOENT (both; lock)
//!   rm ../f                -> rc 0, f gone (base rc 1)
//!   rm -r ../sub           -> rc 0, sub gone (base rc 1)
//!   rm ../sub (no -r)      -> rc 1 "Is a directory" (base: ENOENT
//!                             wording, wrong side)
//!   rm -r ../..            -> rc 1 "refusing to remove '.' or '..'
//!                             directory" (base: bare ENOENT)
//!   rm missing / rm -f     -> rc 1 / rc 0 silent (both; lock)
//!   touch ../t             -> rc 0, t created in parent (base rc 1)
//!   touch ../f             -> rc 0, parent file stamped (base rc 1)
//!   touch -r ../f ../t2    -> rc 0, t2 carries f's times (base
//!                             rc 1 at the reference stat)
//!   touch newfile          -> rc 1 ENOENT (both; lock)
//!   stocked members        -> rm/touch of a name the deleting
//!                             rm -rf already unlinked: rc 1 ENOENT
//!                             (both; lock)
//!   ghost (a recreated in place by an external process):
//!     mkdir newdir         -> rc 1 (base rc 0, created IN ghost)
//!     rm marker            -> rc 1 (base rc 0, DELETED the
//!                             ghost's marker)
//!     touch marker         -> rc 1 (base rc 0, stamped the ghost)
//!
//! These pins assert COMPLETION FORMS (output bytes + exit status +
//! physical landings), never timing. Failure cells assert the
//! failure SIDE (stderr family), not verbatim diagnostic text where
//! the emulation's wording legitimately differs from coreutils
//! (e.g. `mkdir -p` names the first dead component in GNU, the
//! operand word in rubash). Unix only: the cwd anchor is a Unix
//! directory handle; off Unix there is no anchor and the face
//! keeps its pre-fix shape.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

struct ScratchDir(PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A canonicalized scratch root D: D/f = hello, D/sub/sf = world,
/// D/a the directory the shell enters and loses. With `stocked`,
/// D/a carries victim + sub/leaf (for the member-gone locks, whose
/// prelude removes a with an external rm -rf).
fn scratch(case: &str, stocked: bool) -> ScratchDir {
    let dir = std::env::temp_dir().join(format!("rubash-3276-{case}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("a")).expect("create a");
    std::fs::create_dir_all(dir.join("sub")).expect("create sub");
    std::fs::write(dir.join("f"), "hello\n").expect("write f");
    std::fs::write(dir.join("sub/sf"), "world\n").expect("write sf");
    if stocked {
        std::fs::create_dir_all(dir.join("a/sub")).expect("create a/sub");
        std::fs::write(dir.join("a/victim"), "victim\n").expect("write victim");
        std::fs::write(dir.join("a/sub/leaf"), "leaf\n").expect("write leaf");
    }
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

/// The deleted-cwd prelude for an EMPTY a: enter D/a, have a
/// subshell remove it, stay there.
fn prelude() -> &'static str {
    "cd a || exit 99\n(cd ..; rmdir a) || exit 98\n"
}

/// The deleted-cwd prelude for a STOCKED a: an external rm -rf
/// removes it (unlinking the stock first — which is precisely what
/// the member-gone locks measure).
fn prelude_stocked() -> &'static str {
    "cd a || exit 99\n(cd ..; /bin/rm -rf a) || exit 98\n"
}

/// Recreate D/a from OUTSIDE the stationary session (a subshell
/// that walks by the absolute $HOME path) and stock it with a file
/// that exists only in the recreated directory.
const RECREATE: &str = "(cd \"$HOME\" && mkdir a && printf 'ghost\\n' > a/marker) || exit 97\n";

fn landed(dir: &Path, rel: &str) -> String {
    std::fs::read_to_string(dir.join(rel))
        .unwrap_or_else(|error| panic!("expected {rel} to land under {}: {error}", dir.display()))
}

// ---------------------------------------------------------------- mkdir: fail-first

#[test]
fn issue3276_mkdir_escape_to_parent() {
    let dir = scratch("mk1", false);
    let script = format!("{}mkdir ../newdir; echo rc=$?\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr:\n{stderr}");
    assert!(dir.0.join("newdir").is_dir(), "newdir must land in D");
}

#[test]
fn issue3276_mkdir_parents_escape_without_resurrecting() {
    // Base rubash answered rc 0 here too — but by RESURRECTING the
    // deleted directory (create_dir_all rebuilt `a` from the dead
    // string) and creating under it. The landing alone cannot
    // carry the cell: the deleted directory must stay deleted.
    let dir = scratch("mk2", false);
    let script = format!("{}mkdir -p ../pdir/deep; echo rc=$?\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr:\n{stderr}");
    assert!(dir.0.join("pdir/deep").is_dir(), "pdir/deep must land in D");
    assert!(
        !dir.0.join("a").exists(),
        "the deleted directory must not be resurrected"
    );
}

#[test]
fn issue3276_mkdir_parents_into_dead_fails() {
    let dir = scratch("mk3", false);
    let script = format!("{}mkdir -p inner/deeper; echo rc=$?\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr:\n{stderr}");
    assert!(
        stderr.contains("cannot create directory"),
        "failure must be on the create side:\n{stderr}"
    );
    assert!(
        !dir.0.join("a").exists(),
        "the deleted directory must not be resurrected"
    );
}

#[test]
fn issue3276_mkdir_ghost_not_crossed() {
    let dir = scratch("mk4", false);
    let script = format!(
        "{}{}mkdir newdir; echo rc=$?; mkdir ../gdir; echo grc=$?\n",
        prelude(),
        RECREATE
    );
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=1\ngrc=0\n", "stderr:\n{stderr}");
    assert!(dir.0.join("gdir").is_dir(), "gdir must land in D");
    assert!(
        !dir.0.join("a/newdir").exists(),
        "nothing may be created inside the ghost"
    );
    assert_eq!(landed(&dir.0, "a/marker"), "ghost\n", "ghost untouched");
}

// ---------------------------------------------------------------- mkdir: lock

#[test]
fn issue3276_mkdir_into_dead_fails() {
    // GNU fails this shape too (the kernel refuses to create
    // inside the dead directory); base rubash agreed. The cell
    // locks the agreement so the anchor route cannot over-reach.
    let dir = scratch("mk5", false);
    let script = format!("{}mkdir newdir; echo rc=$?\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr:\n{stderr}");
    assert!(
        stderr.contains("cannot create directory"),
        "stderr:\n{stderr}"
    );
    assert!(!dir.0.join("newdir").exists(), "newdir must not land in D");
    assert!(!dir.0.join("a").exists(), "a must stay deleted");
}

// ---------------------------------------------------------------- rm: fail-first

#[test]
fn issue3276_rm_escape_to_parent() {
    let dir = scratch("rm1", false);
    let script = format!("{}rm ../f; echo rc=$?\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr:\n{stderr}");
    assert!(!dir.0.join("f").exists(), "D/f must be removed");
}

#[test]
fn issue3276_rm_tree_escape_to_parent() {
    let dir = scratch("rm2", false);
    let script = format!("{}rm -r ../sub; echo rc=$?\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr:\n{stderr}");
    assert!(!dir.0.join("sub").exists(), "D/sub must be removed");
    assert_eq!(landed(&dir.0, "f"), "hello\n", "D/f must be whole");
}

#[test]
fn issue3276_rm_dir_without_recursive_is_a_directory() {
    // The operand must RESOLVE first (anchor walk), then fail on
    // the type side — base rubash failed at the resolution with a
    // bare ENOENT, the wrong side entirely.
    let dir = scratch("rm3", false);
    let script = format!("{}rm ../sub; echo rc=$?\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr:\n{stderr}");
    assert!(
        stderr.contains("Is a directory"),
        "failure must be the directory refusal:\n{stderr}"
    );
    assert_eq!(landed(&dir.0, "sub/sf"), "world\n", "sub must be whole");
}

#[test]
fn issue3276_rm_dotdot_refusal() {
    // GNU refuses `rm -r ../..` from the dead inode with the
    // `.`/`..` refusal — the guard's existence question is asked
    // of the operand's resolved identity, which the dead string
    // cannot answer (base: guard never fired, bare ENOENT).
    let dir = scratch("rm4", false);
    let script = format!("{}rm -r ../..; echo rc=$?\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr:\n{stderr}");
    assert!(
        stderr.contains("refusing to remove"),
        "the dot/dotdot refusal must fire:\n{stderr}"
    );
    assert_eq!(landed(&dir.0, "f"), "hello\n", "nothing may be removed");
    assert_eq!(
        landed(&dir.0, "sub/sf"),
        "world\n",
        "nothing may be removed"
    );
}

#[test]
fn issue3276_rm_ghost_not_crossed() {
    // Base rubash DELETED the ghost's marker here (rc 0): the
    // session-path string resolved into the recreated directory.
    // GNU's session sits in the dead inode — the marker does not
    // exist for it, and the ghost stays untouched.
    let dir = scratch("rm5", false);
    let script = format!(
        "{}{}rm marker; echo rc=$?; rm ../f; echo frc=$?\n",
        prelude(),
        RECREATE
    );
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=1\nfrc=0\n", "stderr:\n{stderr}");
    assert_eq!(landed(&dir.0, "a/marker"), "ghost\n", "ghost untouched");
    assert!(!dir.0.join("f").exists(), "D/f must be removed");
}

// ---------------------------------------------------------------- rm: locks

#[test]
fn issue3276_rm_missing_and_force() {
    let dir = scratch("rm6", false);
    let script = format!(
        "{}rm missing; echo rc=$?; rm -f missing; echo frc=$?\n",
        prelude()
    );
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=1\nfrc=0\n", "stderr:\n{stderr}");
    assert!(
        stderr.contains("cannot remove 'missing': No such file or directory"),
        "stderr:\n{stderr}"
    );
}

#[test]
fn issue3276_rm_member_gone() {
    // The stocked members were unlinked by the very rm -rf that
    // deleted the cwd — GNU answers plain ENOENT for them, and so
    // did base rubash (through the dead string). Lock: the anchor
    // route must not invent them back.
    let dir = scratch("rm7", true);
    let script = format!(
        "{}rm victim; echo rc=$?; rm -r sub; echo rrc=$?\n",
        prelude_stocked()
    );
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=1\nrrc=1\n", "stderr:\n{stderr}");
    assert_eq!(landed(&dir.0, "f"), "hello\n", "D/f must be whole");
}

// ---------------------------------------------------------------- touch: fail-first

#[test]
fn issue3276_touch_escape_creates_in_parent() {
    let dir = scratch("tc1", false);
    let script = format!("{}touch ../t; echo rc=$?\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr:\n{stderr}");
    assert!(dir.0.join("t").is_file(), "t must land in D");
}

#[test]
fn issue3276_touch_escape_stamps_parent_file() {
    let dir = scratch("tc2", false);
    // Age D/f first; the stamp from the dead session must renew it.
    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(978_307_200); // 2001-01-01
    let file = std::fs::File::options()
        .write(true)
        .open(dir.0.join("f"))
        .expect("open f");
    file.set_times(std::fs::FileTimes::new().set_modified(old))
        .expect("age f");
    drop(file);
    let script = format!("{}touch ../f; echo rc=$?\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr:\n{stderr}");
    let modified = std::fs::metadata(dir.0.join("f"))
        .expect("stat f")
        .modified()
        .expect("mtime f");
    assert!(modified > old, "f's mtime must be renewed by the stamp");
    assert_eq!(landed(&dir.0, "f"), "hello\n", "content untouched");
}

#[test]
fn issue3276_touch_reference_escape() {
    // Both the -r reference AND the operand escape: base rubash
    // died at the reference stat before any operand was reached.
    let dir = scratch("tc3", false);
    let script = format!("{}touch -r ../f ../t2; echo rc=$?\n", prelude());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr:\n{stderr}");
    let f_meta = std::fs::metadata(dir.0.join("f")).expect("stat f");
    let t2_meta = std::fs::metadata(dir.0.join("t2")).expect("stat t2");
    assert_eq!(
        f_meta.modified().expect("f mtime"),
        t2_meta.modified().expect("t2 mtime"),
        "t2 must carry f's mtime"
    );
    assert_eq!(
        f_meta.accessed().expect("f atime"),
        t2_meta.accessed().expect("t2 atime"),
        "t2 must carry f's atime"
    );
}

#[test]
fn issue3276_touch_ghost_not_crossed() {
    // Base rubash stamped the GHOST's marker rc 0 (the session
    // string resolved into the recreated directory). GNU's touch
    // in the dead inode gets the kernel's ENOENT — a create aimed
    // into a dead directory fails — and the ghost's marker keeps
    // its content and its mtime.
    let dir = scratch("tc4", false);
    let stamp_record =
        "(cd \"$HOME\" && /bin/stat -c %Y a/marker > marker-mtime-before) || exit 96\n";
    let script = format!(
        "{}{}{}touch marker; echo rc=$?; (cd \"$HOME\" && /bin/stat -c %Y a/marker > marker-mtime-after) || exit 95\n",
        prelude(),
        RECREATE,
        stamp_record
    );
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr:\n{stderr}");
    assert_eq!(landed(&dir.0, "a/marker"), "ghost\n", "ghost untouched");
    assert_eq!(
        landed(&dir.0, "marker-mtime-before"),
        landed(&dir.0, "marker-mtime-after"),
        "the ghost marker's mtime must not move"
    );
}

// ---------------------------------------------------------------- touch: locks

#[test]
fn issue3276_touch_into_dead_fails() {
    let dir = scratch("tc5", false);
    let script = format!(
        "{}touch newfile; echo rc=$?; touch -c missing; echo crc=$?\n",
        prelude()
    );
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=1\ncrc=0\n", "stderr:\n{stderr}");
    assert!(
        stderr.contains("cannot touch 'newfile': No such file or directory"),
        "stderr:\n{stderr}"
    );
    assert!(!dir.0.join("newfile").exists(), "newfile must not land");
}

#[test]
fn issue3276_touch_member_gone() {
    let dir = scratch("tc6", true);
    let script = format!("{}touch victim; echo rc=$?\n", prelude_stocked());
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr:\n{stderr}");
    assert!(
        stderr.contains("cannot touch 'victim': No such file or directory"),
        "stderr:\n{stderr}"
    );
}

// ---------------------------------------------------------------- live-cwd control

#[test]
fn issue3276_live_cwd_control() {
    // The convergence must leave every ordinary shape on the
    // plain path, byte for byte (plain roots).
    let dir = scratch("live", false);
    let script = "\
mkdir nd; echo mk=$?
mkdir -p nd2/deep; echo mkp=$?
touch tf; echo tc=$?
touch -r f tf2; echo tcr=$?
rm tf; echo rm=$?
rm -r nd2; echo rmr=$?
rmdir nd; echo rd=$?
";
    let (stdout, stderr, code) = run_rubash(&dir.0, script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(
        stdout, "mk=0\nmkp=0\ntc=0\ntcr=0\nrm=0\nrmr=0\nrd=0\n",
        "stderr:\n{stderr}"
    );
    assert!(dir.0.join("tf2").is_file(), "tf2 must land");
    assert!(!dir.0.join("nd").exists(), "nd removed by rmdir");
}
