//! rubash#3394 — `cd` must tilde-expand CDPATH entries before searching.
//!
//! GNU: cd_builtin's CDPATH loop (builtins/cd.def:353-388) joins each
//! colon unit with the operand through
//! `sh_makepath (path, dirname, MP_DOTILDE)` (cd.def:360); makepath
//! expands a `~`-led element with bash_tilde_expand
//! (lib/sh/makepath.c:98), so `CDPATH='~/cdtest'; cd target` searches
//! `$HOME/cdtest/target`, a bare `~` entry searches `$HOME` itself,
//! and a `~/...` entry in any position (first or later) is expanded
//! before the join. The announcement prints the found (joined,
//! expanded) path (cd.def:375-377), and under `-P` only $PWD becomes
//! the physical directory while the announcement keeps the logical
//! joined spelling.
//!
//! rubash joined the unit verbatim, so a `~/...` entry was treated as
//! a relative path: the target was missed when it existed only under
//! the tilde entry (rc 1), and the cwd copy won when the target
//! existed in both places. The fix expands each unit with
//! `expand_tilde_segment` (the bash_tilde_expand port) inside
//! `resolve_cdpath`, before the join — the same call also covers
//! `~+`/`~-`/`~N` and `~user` per the GNU rule, and leaves an
//! unresolvable prefix verbatim (tilde.c:398 savestring fallback).
//!
//! Expectations byte-verified against the GNU bash 5.3.20 oracle with
//! script-file probes (seed_r102a.sh 10/10 plus the trailing-slash,
//! two-place, -P-via-symlink and announcement probes in REPORT.md);
//! the scratch path is normalized to WORK below.

#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue3394-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    for sub in [
        "home/cdtest/target",
        "home/cdtest/onlycdtest",
        "home/target2",
        "home/cde/target",
        "home/cdt/target",
        "home/real/target",
    ] {
        fs::create_dir_all(dir.join(sub)).expect("create scratch subdir");
    }
    dir
}

/// Run `script` as a script FILE whose $1 is the scratch dir; return
/// (stdout, stderr) with the scratch path normalized to WORK. The
/// script itself always exits 0; per-case statuses are echoed inside.
/// Each script sets HOME to $1/home so `~` expands under the scratch.
fn run_script(dir: &Path, script: &str) -> (String, String) {
    let script_path = dir.join("probe.sh");
    fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(RUBASH)
        .arg(&script_path)
        .arg(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    assert!(
        output.status.success(),
        "probe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    (
        String::from_utf8_lossy(&output.stdout)
            .replace(&dir.to_string_lossy().into_owned(), "WORK"),
        String::from_utf8_lossy(&output.stderr)
            .replace(&dir.to_string_lossy().into_owned(), "WORK"),
    )
}

#[test]
fn issue3394_seed_tilde_entry_hit() {
    let dir = scratch("seed");
    let (out, err) = run_script(
        &dir,
        r#"cd "$1" || exit 1
HOME="$1/home"
CDPATH='~/cdtest'
cd target
echo "rc=$? PWD=$PWD"
"#,
    );
    // Announcement: the found (joined, expanded) path.
    assert_eq!(
        out,
        "WORK/home/cdtest/target\n\
         rc=0 PWD=WORK/home/cdtest/target\n"
    );
    assert_eq!(err, "");
}

#[test]
fn issue3394_bare_tilde_entry() {
    let dir = scratch("bare");
    let (out, err) = run_script(
        &dir,
        r#"cd "$1" || exit 1
HOME="$1/home"
CDPATH='~'
cd target2
echo "rc=$? PWD=$PWD"
"#,
    );
    assert_eq!(
        out,
        "WORK/home/target2\n\
         rc=0 PWD=WORK/home/target2\n"
    );
    assert_eq!(err, "");
}

#[test]
fn issue3394_tilde_second_entry() {
    let dir = scratch("second");
    let (out, err) = run_script(
        &dir,
        r#"cd "$1" || exit 1
HOME="$1/home"
CDPATH='/nonexistent:~/cde'
cd target
echo "rc=$? PWD=$PWD"
"#,
    );
    assert_eq!(
        out,
        "WORK/home/cde/target\n\
         rc=0 PWD=WORK/home/cde/target\n"
    );
    assert_eq!(err, "");
}

#[test]
fn issue3394_trailing_slash_entry() {
    let dir = scratch("slash");
    let (out, err) = run_script(
        &dir,
        r#"cd "$1" || exit 1
HOME="$1/home"
CDPATH='~/cdtest/'
cd target
echo "rc=$? PWD=$PWD"
"#,
    );
    assert_eq!(
        out,
        "WORK/home/cdtest/target\n\
         rc=0 PWD=WORK/home/cdtest/target\n"
    );
    assert_eq!(err, "");
}

#[test]
fn issue3394_cdpath_beats_cwd_copy() {
    let dir = scratch("twoplace");
    // A same-named directory in the cwd does NOT win over the
    // CDPATH hit: GNU's CDPATH loop precedes the plain attempt.
    fs::create_dir_all(dir.join("target")).expect("cwd target");
    let (out, err) = run_script(
        &dir,
        r#"cd "$1" || exit 1
HOME="$1/home"
CDPATH='~/cdtest'
cd target
echo "rc=$? PWD=$PWD"
"#,
    );
    assert_eq!(
        out,
        "WORK/home/cdtest/target\n\
         rc=0 PWD=WORK/home/cdtest/target\n"
    );
    assert_eq!(err, "");
}

#[test]
fn issue3394_tilde_entry_physical_dash_p() {
    let dir = scratch("dashp");
    std::os::unix::fs::symlink(dir.join("home/real"), dir.join("home/link"))
        .expect("symlink home/link -> home/real");
    let (out, err) = run_script(
        &dir,
        r#"cd "$1" || exit 1
HOME="$1/home"
CDPATH='~/link'
cd -P target
echo "rc=$? PWD=$PWD"
"#,
    );
    // Announcement keeps the logical joined spelling (through the
    // symlink); $PWD is the physical directory (cd.def:375-377,
    // bindpwd(no_symlinks)).
    assert_eq!(
        out,
        "WORK/home/link/target\n\
         rc=0 PWD=WORK/home/real/target\n"
    );
    assert_eq!(err, "");
}

#[test]
fn issue3394_controls_absolute_entry_and_tilde_arg() {
    let dir = scratch("controls");
    let (out, err) = run_script(
        &dir,
        r#"cd "$1" || exit 1
HOME="$1/home"
CDPATH="$HOME/cdt"
cd target
echo "absolute-entry: rc=$? PWD=$PWD"
cd "$1" || exit 1
cd ~/cdtest/target
echo "tilde-arg: rc=$? PWD=$PWD"
"#,
    );
    assert_eq!(
        out,
        "WORK/home/cdt/target\n\
         absolute-entry: rc=0 PWD=WORK/home/cdt/target\n\
         tilde-arg: rc=0 PWD=WORK/home/cdtest/target\n"
    );
    assert_eq!(err, "");
}

#[test]
fn issue3394_miss_falls_back_to_cwd_and_unresolvable_user_verbatim() {
    let dir = scratch("miss");
    // `cwdtarget` exists ONLY in the cwd: the genuine-CDPATH-miss
    // cwd fallback is observable on it. `onlycdtest` exists ONLY
    // under the tilde entry: it must stay unreachable through an
    // unresolvable `~user` entry (no cwd copy to fall back to).
    fs::create_dir_all(dir.join("cwdtarget")).expect("cwd target");
    let (out, err) = run_script(
        &dir,
        r#"cd "$1" || exit 1
HOME="$1/home"
CDPATH='~/cdtest'
cd nosuchdir; echo "miss: rc=$? PWD=$PWD"
cd cwdtarget; echo "cwd-fallback: rc=$? PWD=$PWD"
cd "$1" || exit 1
CDPATH='~nosuchuser3394/cdtest'
cd onlycdtest; echo "unresolvable-user: rc=$? PWD=$PWD"
"#,
    );
    // Genuine CDPATH miss: rc 1, $PWD unchanged, then the plain
    // cwd attempt still works; an unresolvable `~user` entry keeps
    // the unit verbatim (no expansion, no crash) and misses.
    assert_eq!(
        out,
        "miss: rc=1 PWD=WORK\n\
         cwd-fallback: rc=0 PWD=WORK/cwdtarget\n\
         unresolvable-user: rc=1 PWD=WORK\n"
    );
    assert!(
        err.contains("cd: nosuchdir: No such file or directory"),
        "stderr: {err}"
    );
    assert!(
        err.contains("cd: onlycdtest: No such file or directory"),
        "stderr: {err}"
    );
}
