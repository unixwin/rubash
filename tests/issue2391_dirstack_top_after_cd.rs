//! rubash#2391 — after `pushd` builds the stack, a plain `cd` must
//! refresh the stack top: DIRSTACK[0] is the current directory.
//!
//! GNU never stores the current directory in the stack list at all:
//! builtins/pushd.def:669 get_directory_stack appends
//! get_working_directory after the pushed entries, dirs_builtin
//! (pushd.def, dirs) prints it first ("The first directory printed is
//! always the current working directory"), and variables.c:1619
//! get_dirstack rebuilds the DIRSTACK array from get_directory_stack,
//! so every read face — `dirs`, `dirs -v`, `dirs -p`, ${DIRSTACK[0]},
//! ${DIRSTACK[@]}, ~+0 — shows the live cwd as element 0. rubash
//! serializes the stack with the cwd in slot 0 (__RUBASH_DIR_STACK),
//! so the cd commit must re-bind slot 0; a stack built by pushd
//! otherwise keeps displaying the pre-cd top while $PWD has moved on.
//!
//! Expectations byte-verified against the GNU bash 5.3.20 oracle with
//! script-file probes (cluster workdir probes/p1_seed.sh, p2_forms.sh,
//! p3_cdpath.sh, p4_more.sh; the scratch path is normalized to WORK).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue2391-{tag}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    for sub in ["a", "b", "x/y"] {
        fs::create_dir_all(dir.join(sub)).expect("create scratch subdir");
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
    String::from_utf8_lossy(&output.stdout)
        .replace(&dir.to_string_lossy().into_owned(), "WORK")
}

#[test]
fn issue2391_seed_pushd_pushd_cd() {
    let dir = scratch("seed");
    let out = run_script(
        &dir,
        r#"cd "$1" || exit 1
pushd a >/dev/null
pushd ../b >/dev/null
cd ../a
echo "dirs: $(dirs)"
echo "pwd: $PWD"
echo "ds0: ${DIRSTACK[0]}"
dirs -v
dirs -p
echo "all: ${DIRSTACK[@]}"
"#,
    );
    assert_eq!(
        out,
        "dirs: WORK/a WORK/a WORK\n\
         pwd: WORK/a\n\
         ds0: WORK/a\n\
         \x200  WORK/a\n\
         \x201  WORK/a\n\
         \x202  WORK\n\
         WORK/a\n\
         WORK/a\n\
         WORK\n\
         all: WORK/a WORK/a WORK\n"
    );
}

#[test]
fn issue2391_cd_plain_and_modes_refresh_top() {
    let dir = scratch("modes");
    let out = run_script(
        &dir,
        r#"cd "$1" || exit 1
pushd a >/dev/null
cd ../b
echo "plain: $(dirs)"
echo "ds0: ${DIRSTACK[0]}"
cd -P ../a
echo "phys: $(dirs)"
cd -L ../b
echo "logical: $(dirs)"
"#,
    );
    assert_eq!(
        out,
        "plain: WORK/b WORK\n\
         ds0: WORK/b\n\
         phys: WORK/a WORK\n\
         logical: WORK/b WORK\n"
    );
}

#[test]
fn issue2391_controls_popd_and_cd_dash() {
    let dir = scratch("controls");
    let out = run_script(
        &dir,
        r#"cd "$1" || exit 1
cd a
echo "nopushd: $(dirs)"
cd ..
pushd a >/dev/null
cd ../b
popd >/dev/null
echo "after-popd: $(dirs)"
pushd b >/dev/null
cd ../a
cd - >/dev/null
echo "after-cd-dash: $(dirs)"
"#,
    );
    assert_eq!(
        out,
        "nopushd: WORK/a\n\
         after-popd: WORK\n\
         after-cd-dash: WORK/b WORK\n"
    );
}

#[test]
fn issue2391_swap_subshell_and_pushd_dot() {
    let dir = scratch("swap");
    let out = run_script(
        &dir,
        r#"cd "$1" || exit 1
pushd a >/dev/null
pushd ../b >/dev/null
pushd >/dev/null
echo "after-swap: $(dirs)"
cd ../b
echo "swap-then-cd: $(dirs)"
(cd ../a; echo "in-sub: $(dirs)")
echo "outer: $(dirs)"
pushd . >/dev/null
cd ..
echo "pushd-dot-cdup: $(dirs)"
"#,
    );
    assert_eq!(
        out,
        "after-swap: WORK/a WORK/b WORK\n\
         swap-then-cd: WORK/b WORK/b WORK\n\
         in-sub: WORK/a WORK/b WORK\n\
         outer: WORK/b WORK/b WORK\n\
         pushd-dot-cdup: WORK WORK/b WORK/b WORK\n"
    );
}

#[test]
fn issue2391_cdpath_and_cdable_vars_refresh_top() {
    let dir = scratch("cdpath");
    let out = run_script(
        &dir,
        r#"cd "$1" || exit 1
pushd b >/dev/null
CDPATH="$1/x"
cd y
echo "cdpath: $(dirs)"
echo "ds0: ${DIRSTACK[0]}"
shopt -s cdable_vars
v="$1/a"
cd v >/dev/null
echo "cdable: $(dirs)"
"#,
    );
    assert_eq!(
        out,
        "WORK/x/y\n\
         cdpath: WORK/x/y WORK\n\
         ds0: WORK/x/y\n\
         cdable: WORK/a WORK\n"
    );
}

#[test]
fn issue2391_tilde_and_declare_read_top() {
    let dir = scratch("tilde");
    let out = run_script(
        &dir,
        r#"cd "$1" || exit 1
pushd a >/dev/null
cd ../b
echo "tilde0:" ~+0
echo "tilde1:" ~+1
declare -p DIRSTACK
"#,
    );
    assert_eq!(
        out,
        "tilde0: WORK/b\n\
         tilde1: WORK\n\
         declare -a DIRSTACK=([0]=\"WORK/b\" [1]=\"WORK\")\n"
    );
}
