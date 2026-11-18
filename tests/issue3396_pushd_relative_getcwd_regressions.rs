//! rubash#3396 — `pushd` must resolve a relative target against the
//! real cwd (getcwd), not $PWD: with PWD unset, empty, or stale the
//! relative pushd must still succeed and rebuild PWD to the real path.
//!
//! GNU: pushd_builtin delegates the Dir operand to cd_builtin
//! (builtins/pushd.def:296-302), whose change_to_directory
//! (builtins/cd.def:522ff) chdirs the operand as-is — the kernel
//! resolves a relative operand against the real cwd, never $PWD — and
//! bindpwd then rebuilds PWD from get_working_directory
//! (builtins/common.c:592, getcwd). Under G-02 the session cwd field is
//! that real cwd, so the resolution base is the VALIDATED logical pwd
//! ($PWD only while it still names the working directory, the physical
//! session cwd otherwise — cd/paths.rs validated_logical_pwd, #2143).
//! rubash's normalize_stack_dir instead joined every relative operand
//! onto the raw $PWD, so `unset PWD; pushd a` looked up `/a` and
//! `PWD=/wrong; pushd a` looked up `/wrong/a` (rc 1, PWD left
//! empty/stale, stack untouched).
//!
//! Expectations byte-verified against the GNU bash oracle with
//! script-file probes (seed_r104a.sh, 10/10 on GNU 5.2.21 here and
//! 5.3.20 in the verdict; the extended shapes below were probed the
//! same way). Scratch path normalized to WORK below.

#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue3396-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    for name in ["a", "b", "c", "d", "e"] {
        fs::create_dir_all(dir.join(name)).expect("create scratch subdir");
    }
    dir
}

/// Run `script` as a script FILE whose $1 is the scratch dir; return
/// stdout with the scratch path normalized to WORK.
fn run_script(dir: &Path, script: &str) -> String {
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
    let scratch = dir.to_string_lossy().into_owned();
    String::from_utf8_lossy(&output.stdout)
        .replace(&format!("/{scratch}"), "//WORK")
        .replace(&scratch, "WORK")
}

#[test]
fn issue3396_pushd_relative_pwd_unset() {
    let dir = scratch("unset");
    let out = run_script(
        &dir,
        r#"cd "$1" || exit 1
unset PWD
pushd a >/dev/null; echo "rc=$? PWD=$PWD"
echo "top: ${DIRSTACK[0]}"
dirs -l
popd >/dev/null; echo "back: rc=$? PWD=$PWD"
"#,
    );
    assert_eq!(
        out,
        "rc=0 PWD=WORK/a\n\
         top: WORK/a\n\
         WORK/a WORK\n\
         back: rc=0 PWD=WORK\n"
    );
}

#[test]
fn issue3396_pushd_relative_pwd_empty() {
    let dir = scratch("empty");
    let out = run_script(
        &dir,
        r#"cd "$1" || exit 1
PWD=
pushd b >/dev/null; echo "rc=$? PWD=$PWD"
echo "top: ${DIRSTACK[0]}"
"#,
    );
    assert_eq!(out, "rc=0 PWD=WORK/b\ntop: WORK/b\n");
}

#[test]
fn issue3396_pushd_relative_pwd_stale() {
    let dir = scratch("stale");
    let out = run_script(
        &dir,
        r#"cd "$1" || exit 1
PWD=/wrong
pushd c >/dev/null; echo "rc=$? PWD=$PWD"
echo "top: ${DIRSTACK[0]}"
"#,
    );
    assert_eq!(out, "rc=0 PWD=WORK/c\ntop: WORK/c\n");
}

#[test]
fn issue3396_pushd_relative_dot_slash_and_chain() {
    let dir = scratch("chain");
    let out = run_script(
        &dir,
        r#"cd "$1" || exit 1
PWD=/wrong
pushd ./a >/dev/null; echo "first: rc=$? PWD=$PWD"
pushd ../b >/dev/null; echo "second: rc=$? PWD=$PWD"
dirs -l
popd >/dev/null; echo "pop1: rc=$? PWD=$PWD"
popd >/dev/null; echo "pop2: rc=$? PWD=$PWD"
"#,
    );
    assert_eq!(
        out,
        "first: rc=0 PWD=WORK/a\n\
         second: rc=0 PWD=WORK/b\n\
         WORK/b WORK/a WORK\n\
         pop1: rc=0 PWD=WORK/a\n\
         pop2: rc=0 PWD=WORK\n"
    );
}

#[test]
fn issue3396_pushd_nocd_unset_pwd_control() {
    // Control: `pushd -n` never calls cd, so it stores the RAW operand
    // and leaves PWD untouched (pushd.def:301) — the base change must
    // not touch this arm.
    let dir = scratch("nocd");
    let out = run_script(
        &dir,
        r#"cd "$1" || exit 1
unset PWD
pushd -n a >/dev/null; echo "rc=$? PWD=${PWD-unset}"
dirs -l
"#,
    );
    assert_eq!(out, "rc=0 PWD=unset\nWORK a\n");
}

#[test]
fn issue3396_controls_cd_and_absolute_pushd() {
    // Controls (already agreeing per the verdict): plain cd re-derives
    // from getcwd with PWD unset, and an absolute pushd target never
    // consults $PWD.
    let dir = scratch("controls");
    let out = run_script(
        &dir,
        r#"cd "$1" || exit 1
unset PWD
cd d; echo "cd: rc=$? PWD=$PWD"
cd "$1"
unset PWD
pushd "$1/e" >/dev/null; echo "abs: rc=$? PWD=$PWD"
"#,
    );
    assert_eq!(out, "cd: rc=0 PWD=WORK/d\nabs: rc=0 PWD=WORK/e\n");
}
