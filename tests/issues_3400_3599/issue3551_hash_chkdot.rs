//! Regression pins for rubash#3551 — the command hash table ignored
//! HASH_CHKDOT (hashcmd.h:34). GNU findcmd.c:550-551 sets the global
//! `dot_found_in_search` when a `.`-leading PATH element resolves to the
//! session cwd (same_file), evaluated per element in PATH order *before*
//! the element's hit test; findcmd.c:632 resets it at walk entry.
//! phash_insert consumers then store it in the entry (findcmd.c:416-426
//! for walk resolutions, hash.def:225 for `hash NAME`, but a literal 0 at
//! hash.def:195 for `hash -p` and variables.c:1739 for BASH_CMDS writes).
//!
//! phash_search (hashcmd.c:146-191) applies the CHKDOT/RELPATH arm on
//! every serve: `tail` is the stored path when relative (RELPATH,
//! general.h:267) else the command name; `./tail` wins while executable.
//! A `.`-leading stored path resolving to `.` unexecutably is a miss and
//! the lookup falls through to a PATH walk, whose phash_insert REPLACES
//! the stale entry (findcmd.c:416-426).
//!
//! All expectations are the GNU bash 5.3.20 outputs for the same probe
//! scripts under LC_ALL=C.UTF-8. The execution-driven cells need POSIX
//! `#!/bin/sh` fixtures and `/bin/cp`, so they stay unix-only; the
//! Windows listing walk (`find_in_path_via_listings`) records the same
//! CHKDOT verdict (rubash#3551 F1) and is pinned by the `type` / `hash -t`
//! cells below, which never execute the fixture.

use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3551-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

struct Outcome {
    stdout: String,
    stderr: String,
    rc: i32,
}

fn run(root: &Path, body: &str) -> Outcome {
    std::fs::write(root.join("case.sh"), body).expect("case script");
    let out = Command::new(BIN)
        .arg("case.sh")
        .current_dir(root)
        .env("LC_ALL", "C.UTF-8")
        .env_remove("BASH_ENV")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    Outcome {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        rc: out.status.code().unwrap_or(-1),
    }
}

/// `bindir/foo` (BIN-FOO) plus a `dotfoo` source the script can install as
/// `./foo` mid-run via absolute `/bin/cp` (the probe PATH carries no
/// /usr/bin on purpose).
fn fixture(tag: &str) -> PathBuf {
    let root = scratch(tag);
    let bin = root.join("bindir");
    std::fs::create_dir_all(&bin).expect("bindir");
    for (dir, name, text) in [
        (&bin, "foo", "#!/bin/sh\necho BIN-FOO\n"),
        (&root, "dotfoo", "#!/bin/sh\necho DOT-FOO\n"),
        (&root, "dotx", "#!/bin/sh\necho DOT-X\n"),
    ] {
        let f = dir.join(name);
        std::fs::write(&f, text).expect("fixture");
        // Windows has no execute bit; `is_executable_file` is `is_file()`
        // there, so a plain file is already a command candidate.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).expect("mode");
        }
    }
    root
}

/// `.` element evaluated BEFORE the hit: `hash foo` misses `./foo` (absent
/// at hash time), but the `.` element still sets dot_found_in_search
/// (findcmd.c:550-551 runs same_file before the hit test), so the recorded
/// bindir path carries HASH_CHKDOT. Once `./foo` becomes executable,
/// phash_search serves `./foo` ahead of the stored path and `type` /
/// `hash -t` report the dotted spelling — the stored entry stays bindir.
// POSIX exec faces (`#!/bin/sh` fixtures, /bin/cp, /bin/chmod).
#[cfg(unix)]
#[test]
fn dot_element_before_hit_marks_chkdot_and_dotted_serve_wins() {
    let root = fixture("before");
    let out = run(
        &root,
        "PATH=\".:$PWD/bindir\"\nhash foo\nhash -l\nfoo\n\
         /bin/cp dotfoo ./foo && /bin/chmod 755 ./foo\n\
         foo\ntype foo\nhash -t foo\nhash -l\n",
    );
    let bindir = root.display();
    assert_eq!(
        out.stdout,
        format!(
            "builtin hash -p {bindir}/bindir/foo foo\n\
             BIN-FOO\n\
             DOT-FOO\n\
             foo is hashed (./foo)\n\
             ./foo\n\
             builtin hash -p {bindir}/bindir/foo foo\n"
        ),
        "stdout={:?}",
        out.stdout
    );
    assert_eq!(out.stderr, "");
    assert_eq!(out.rc, 0);
}

/// `.` element AFTER the hit: PATH order matters — the walk stops at the
/// bindir hit before evaluating `.`, so dot_found_in_search stays 0 and
/// the entry has no CHKDOT. A later `./foo` is never served.
// POSIX exec faces (`#!/bin/sh` fixtures, /bin/cp, /bin/chmod).
#[cfg(unix)]
#[test]
fn dot_element_after_hit_does_not_mark_chkdot() {
    let root = fixture("after");
    let out = run(
        &root,
        "PATH=\"$PWD/bindir:.\"\nhash foo\nhash -l\n\
         /bin/cp dotfoo ./foo && /bin/chmod 755 ./foo\n\
         foo\ntype foo\n",
    );
    let bindir = root.display();
    assert_eq!(
        out.stdout,
        format!(
            "builtin hash -p {bindir}/bindir/foo foo\n\
             BIN-FOO\n\
             foo is hashed ({bindir}/bindir/foo)\n"
        ),
        "stdout={:?}",
        out.stdout
    );
    assert_eq!(out.stderr, "");
    assert_eq!(out.rc, 0);
}

/// `hash -p PATH NAME` hardcodes check_dot=0 (hash.def:195): even with a
/// stale dot_found_in_search left by an earlier walk AND an executable
/// `./x` in the cwd, the pinned entry serves the stored spelling
/// verbatim — here a dead absolute path failing ENOENT with status 127.
// POSIX exec faces (`#!/bin/sh` fixtures, /bin/cp, /bin/chmod).
#[cfg(unix)]
#[test]
fn hash_p_insert_never_carries_dot_verdict() {
    let root = fixture("pinned");
    let out = run(
        &root,
        "PATH=\".:$PWD/bindir\"\nhash foo\nhash -p /gone/x x\n\
         /bin/cp dotx ./x && /bin/chmod 755 ./x\n\
         x\necho \"rc=$?\"\n",
    );
    assert_eq!(out.stdout, "rc=127\n", "stdout={:?}", out.stdout);
    assert_eq!(
        out.stderr, "case.sh: line 5: /gone/x: No such file or directory\n",
        "stderr={:?}",
        out.stderr
    );
}

/// A stored RELPATH entry whose `.`-dir is the session cwd but is not
/// executable (`hash -p ./gone foo`) counts as not found in phash_search
/// (hashcmd.c:183-196): the lookup falls through to a PATH walk and the
/// walk's phash_insert replaces the dead entry.
// POSIX exec faces (`#!/bin/sh` fixtures, /bin/cp, /bin/chmod).
#[cfg(unix)]
#[test]
fn dead_relpath_entry_falls_through_and_is_replaced() {
    let root = fixture("relpath");
    let out = run(
        &root,
        "PATH=\".:$PWD/bindir\"\nhash -p ./gone foo\nfoo\nhash -l\n",
    );
    assert_eq!(
        out.stdout,
        format!(
            "BIN-FOO\nbuiltin hash -p {}/bindir/foo foo\n",
            root.display()
        ),
        "stdout={:?}",
        out.stdout
    );
    assert_eq!(out.stderr, "");
    assert_eq!(out.rc, 0);
}

/// Windows face of the same CHKDOT chain (rubash#3551 F1): the listing
/// walk records the `.`-element verdict, `hash` stores it in the entry,
/// and `phash_search` serves `./NAME` while that spelling is a live
/// command candidate. None of the cells execute the fixture — the
/// assertion is the resolved spelling `type` / `hash -t` report.
#[cfg(not(unix))]
#[test]
fn windows_listing_walk_records_chkdot_and_serves_dot_name() {
    let root = fixture("win-before");
    let out = run(
        &root,
        "PATH=\".:$PWD/bindir\"\nhash foo\n\
         echo dot > ./foo\n\
         type foo\nhash -t foo\nhash -l\n",
    );
    assert!(
        out.stdout.contains("foo is hashed (./foo)"),
        "stdout={:?}",
        out.stdout
    );
    assert!(out.stdout.contains("\n./foo\n"), "stdout={:?}", out.stdout);
    assert!(
        out.stdout.contains("builtin hash -p "),
        "stdout={:?}",
        out.stdout
    );
    assert!(
        out.stdout.contains("/bindir/foo foo\n"),
        "stdout={:?}",
        out.stdout
    );
    assert_eq!(out.stderr, "");
    assert_eq!(out.rc, 0);
}

/// A `.` element AFTER the hit is never examined (the walk stops at the
/// bindir hit), so no CHKDOT is recorded and `./foo` is never served.
#[cfg(not(unix))]
#[test]
fn windows_dot_element_after_hit_stays_unhashed() {
    let root = fixture("win-after");
    let out = run(
        &root,
        "PATH=\"$PWD/bindir:.\"\nhash foo\n\
         echo dot > ./foo\n\
         type foo\nhash -t foo\n",
    );
    assert!(
        out.stdout.contains("foo is hashed ("),
        "stdout={:?}",
        out.stdout
    );
    assert!(!out.stdout.contains("(./foo)"), "stdout={:?}", out.stdout);
    assert!(!out.stdout.contains("\n./foo\n"), "stdout={:?}", out.stdout);
    assert!(
        out.stdout.contains("/bindir/foo\n"),
        "stdout={:?}",
        out.stdout
    );
    assert_eq!(out.stderr, "");
    assert_eq!(out.rc, 0);
}

/// `hash -p` hardcodes check_dot=0 (hash.def:195): even right after a
/// walk that left dot_found_in_search set, a pinned entry serves its
/// stored spelling verbatim — never `./NAME`.
#[cfg(not(unix))]
#[test]
fn windows_hash_p_insert_never_carries_dot_verdict() {
    let root = fixture("win-pinned");
    let out = run(
        &root,
        "PATH=\".:$PWD/bindir\"\nhash foo\nhash -p ./gone/x x\n\
         echo dot > ./x\nhash -t x\n",
    );
    assert!(out.stdout.contains("./gone/x\n"), "stdout={:?}", out.stdout);
    assert_eq!(out.stderr, "");
    assert_eq!(out.rc, 0);
}

/// A dead RELPATH entry (`hash -p ./gone foo`) is refused by
/// phash_search (hashcmd.c:176-187: its `.`-dir is the cwd): `hash -t`
/// reports not found while `type` falls through to the PATH walk and
/// resolves the live command. (Replacing the table entry needs the
/// resolution that only an execution performs — that half stays on the
/// unix cells, which run the fixture.)
#[cfg(not(unix))]
#[test]
fn windows_dead_relpath_entry_falls_through() {
    let root = fixture("win-relpath");
    let out = run(
        &root,
        "PATH=\".:$PWD/bindir\"\nhash -p ./gone foo\n\
         hash -t foo\necho \"rc=$?\"\ntype foo\n",
    );
    assert!(
        out.stderr.contains("hash: foo: not found"),
        "stderr={:?}",
        out.stderr
    );
    assert!(out.stdout.contains("rc=1\n"), "stdout={:?}", out.stdout);
    assert!(
        out.stdout.contains("/bindir/foo\n"),
        "stdout={:?}",
        out.stdout
    );
    assert!(!out.stdout.contains("./gone"), "stdout={:?}", out.stdout);
}
