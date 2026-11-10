//! rubash#3196 — `pushd <dir>` must search $CDPATH exactly like `cd`.
//!
//! GNU: pushd_builtin routes the Dir operand through cd_builtin
//! (builtins/pushd.def:296-302), and cd_builtin's CDPATH loop
//! (builtins/cd.def:353-388) runs BEFORE the plain operand attempt:
//! for a non-absolute operand (general.c absolute_pathname: `/...`,
//! `.`/`./...` and `..`/`../...` are "absolute" and skip the search),
//! each colon unit is joined with the operand (sh_makepath) and tried
//! in order; an empty unit means the current directory. A hit through
//! a NON-empty unit echoes the found directory to stdout (cd.def
//! POSIX step 5 note) before pushd's own dirs listing; an empty-unit
//! hit echoes nothing. The same delegation also carries cdable_vars
//! (cd.def LCD_DOVARS, after the direct attempt fails).
//!
//! rubash's CDPATH search lives only in src/builtins/cd.rs
//! (resolve_cdpath); pushd probed the literal operand only, so a
//! target existing solely under a CDPATH entry failed with
//! `No such file or directory`, rc 1, $PWD unchanged.
//!
//! Expectations byte-verified against the GNU bash 5.3.20 oracle with
//! script-file probes (scratch path normalized to WORK below).

#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue3196-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    for sub in ["sub/target", "s1", "s2/target", "base/sub2/target", "here"] {
        fs::create_dir_all(dir.join(sub)).expect("create scratch subdir");
    }
    dir
}

/// Run `script` as a script FILE whose $1 is the scratch dir; return
/// (stdout, stderr) with the scratch path normalized to WORK. The
/// script itself always exits 0; per-case statuses are echoed inside.
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
fn issue3196_seed_plain_cdpath() {
    let dir = scratch("seed");
    let (out, err) = run_script(
        &dir,
        r#"cd "$1" || exit 1
CDPATH=sub
pushd target
echo "rc=$? PWD=$PWD"
"#,
    );
    // First line: cd's CDPATH announcement (the found path). Second:
    // pushd's dirs listing.
    assert_eq!(
        out,
        "WORK/sub/target\n\
         WORK/sub/target WORK\n\
         rc=0 PWD=WORK/sub/target\n"
    );
    assert_eq!(err, "");
}

#[test]
fn issue3196_unit_forms() {
    let dir = scratch("units");
    let (out, _) = run_script(
        &dir,
        r#"cd "$1" || exit 1
CDPATH=sub:
pushd target >/dev/null; echo "trailing-empty: rc=$? PWD=$PWD"; popd >/dev/null
CDPATH=:sub
pushd target >/dev/null; echo "leading-empty: rc=$? PWD=$PWD"; popd >/dev/null
CDPATH=s1:s2
pushd target >/dev/null; echo "second-entry: rc=$? PWD=$PWD"; popd >/dev/null
CDPATH="$1/sub"
pushd target >/dev/null; echo "absolute-entry: rc=$? PWD=$PWD"; popd >/dev/null
"#,
    );
    assert_eq!(
        out,
        "trailing-empty: rc=0 PWD=WORK/sub/target\n\
         leading-empty: rc=0 PWD=WORK/sub/target\n\
         second-entry: rc=0 PWD=WORK/s2/target\n\
         absolute-entry: rc=0 PWD=WORK/sub/target\n"
    );
}

#[test]
fn issue3196_cdpath_beats_cwd_and_slash_operand() {
    let dir = scratch("order");
    // A same-named directory in the cwd does NOT win over an earlier
    // CDPATH hit: GNU's CDPATH loop precedes the plain attempt.
    fs::create_dir_all(dir.join("target")).expect("cwd target");
    let (out, _) = run_script(
        &dir,
        r#"cd "$1" || exit 1
CDPATH=sub
pushd target >/dev/null; echo "cdpath-first: rc=$? PWD=$PWD"; popd >/dev/null
CDPATH=base
pushd sub2/target
echo "slash-operand: rc=$? PWD=$PWD"
"#,
    );
    assert_eq!(
        out,
        "cdpath-first: rc=0 PWD=WORK/sub/target\n\
         WORK/base/sub2/target\n\
         WORK/base/sub2/target WORK\n\
         slash-operand: rc=0 PWD=WORK/base/sub2/target\n"
    );
}

#[test]
fn issue3196_empty_unit_hit_is_silent() {
    let dir = scratch("emptyunit");
    fs::create_dir_all(dir.join("target")).expect("cwd target");
    let (out, _) = run_script(
        &dir,
        r#"cd "$1" || exit 1
CDPATH=:sub
pushd target
echo "rc=$? PWD=$PWD"
"#,
    );
    // Empty unit = cwd: the hit prints no announcement, only the
    // dirs listing follows.
    assert_eq!(out, "WORK/target WORK\nrc=0 PWD=WORK/target\n");
}

#[test]
fn issue3196_controls_miss_dotled_and_nocd() {
    let dir = scratch("controls");
    let (out, err) = run_script(
        &dir,
        r#"cd "$1" || exit 1
CDPATH=sub
pushd nosuchdir; echo "miss: rc=$? PWD=$PWD"
pushd ./target; echo "dotled: rc=$? PWD=$PWD"
pushd -n target; echo "nocd: rc=$? PWD=$PWD"
dirs -c
pushd here
echo "plain: rc=$? PWD=$PWD"
"#,
    );
    assert_eq!(
        out,
        "miss: rc=1 PWD=WORK\n\
         dotled: rc=1 PWD=WORK\n\
         WORK target\n\
         nocd: rc=0 PWD=WORK\n\
         WORK/here WORK\n\
         plain: rc=0 PWD=WORK/here\n"
    );
    assert!(
        err.contains("pushd: nosuchdir: No such file or directory"),
        "stderr: {err}"
    );
    assert!(
        err.contains("pushd: ./target: No such file or directory"),
        "stderr: {err}"
    );
}

#[test]
fn issue3196_cdable_vars_through_pushd() {
    let dir = scratch("cdable");
    let (out, _) = run_script(
        &dir,
        r#"cd "$1" || exit 1
shopt -s cdable_vars
mydir="$1/sub/target"
pushd mydir
echo "rc=$? PWD=$PWD"
"#,
    );
    // cd_builtin's LCD_DOVARS arm echoes the variable's raw value,
    // then pushd's dirs listing follows.
    assert_eq!(
        out,
        "WORK/sub/target\n\
         WORK/sub/target WORK\n\
         rc=0 PWD=WORK/sub/target\n"
    );
}
