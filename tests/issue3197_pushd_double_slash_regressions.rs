//! rubash#3197 — `pushd //` must preserve the exactly-two-slashes
//! prefix in $PWD and on the directory stack, like `cd //` does.
//!
//! GNU: pushd_builtin delegates the Dir operand to cd_builtin
//! (builtins/pushd.def:296-302), whose change_to_directory
//! (builtins/cd.def:522ff) canonicalizes through sh_canonpath;
//! POSIX leaves exactly two leading slashes implementation-defined
//! and GNU preserves them (`cd //` binds PWD=`//`, `cd ..` from
//! `//tmp` yields `//`), while three or more collapse to one. cd's
//! logical face in rubash already preserves the prefix
//! (cd/paths.rs preserve_leading_double_slash, rubash#2211), but
//! pushd's own normalize_stack_dir rebuilt the path from components
//! under a hardcoded single-"/" prefix, so `//` could never survive.
//!
//! Expectations byte-verified against the GNU bash 5.3.20 oracle with
//! script-file probes (scratch path normalized to WORK below).

#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue3197-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("sub")).expect("create scratch subdir");
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
    // Normalize the scratch path to WORK. The scratch spelling itself
    // starts with `/`, so a preserved `//` prefix in a value fuses with
    // it into one string (`//` + `/tmp/…`); replacing the bare scratch
    // first would match from the second slash and eat the very prefix
    // this pin exists to verify. Replace the fused form first.
    let scratch = dir.to_string_lossy().into_owned();
    String::from_utf8_lossy(&output.stdout)
        .replace(&format!("/{scratch}"), "//WORK")
        .replace(&scratch, "WORK")
}

#[test]
fn issue3197_pushd_double_slash_root() {
    let dir = scratch("root");
    let out = run_script(
        &dir,
        r#"cd "$1" || exit 1
pushd //
echo "rc=$? PWD=$PWD"
echo "top: ${DIRSTACK[0]}"
popd >/dev/null
echo "after-popd: PWD=$PWD"
"#,
    );
    assert_eq!(
        out,
        "// WORK\n\
         rc=0 PWD=//\n\
         top: //\n\
         after-popd: PWD=WORK\n"
    );
}

#[test]
fn issue3197_controls_triple_and_single() {
    let dir = scratch("controls");
    let out = run_script(
        &dir,
        r#"cd "$1" || exit 1
pushd /// >/dev/null; echo "triple: PWD=$PWD"; popd >/dev/null
pushd / >/dev/null; echo "single: PWD=$PWD"; popd >/dev/null
cd //; echo "cd-control: PWD=$PWD"; cd "$1"
"#,
    );
    assert_eq!(
        out,
        "triple: PWD=/\n\
         single: PWD=/\n\
         cd-control: PWD=//\n"
    );
}

#[test]
fn issue3197_double_slash_with_components() {
    let dir = scratch("components");
    let out = run_script(
        &dir,
        r#"cd "$1" || exit 1
pushd "/$1/sub" >/dev/null; echo "path: PWD=$PWD top=${DIRSTACK[0]}"; popd >/dev/null
pushd //tmp/../tmp >/dev/null; echo "dotdot: PWD=$PWD"; popd >/dev/null
cd //
pushd tmp >/dev/null; echo "relative-from-dbl: PWD=$PWD"; popd >/dev/null
"#,
    );
    assert_eq!(
        out,
        "path: PWD=//WORK/sub top=//WORK/sub\n\
         dotdot: PWD=//tmp\n\
         relative-from-dbl: PWD=//tmp\n"
    );
}
