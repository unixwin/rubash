//! rubash#2434 — after `unset DIRSTACK`, no read face may resurrect
//! the directory stack.
//!
//! GNU: unset removes the dynamic SHELL_VAR itself
//! (variables.c:3839 unbind_variable, via unset_builtin), and
//! variables.c:1197-1198 documents that a special variable so unset
//! "loses its special meaning, even if you subsequently set it". The
//! unbind is permanent for the shell's lifetime: every face reads
//! empty, pushd/popd keep maintaining the real stack for `dirs`, and
//! later binds create an ordinary variable. Before the fix, rubash
//! only gated the bare-scalar faces on the unbind mark; the braced
//! faces and the array views read the carrier directly and the old
//! stack came back.
//!
//! Expectations byte-verified against the GNU bash 5.3.20 oracle; the
//! scratch path is normalized to WORK, mirroring tests/issue2407_*.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2434-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    for sub in ["a", "b"] {
        fs::create_dir_all(dir.join(sub)).expect("create scratch subdir");
    }
    dir
}

fn run_script(dir: &Path, script: &str) -> (String, String) {
    let script_path = dir.join("probe.sh");
    fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(RUBASH)
        .arg(&script_path)
        .arg(dir)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .output()
        .expect("run rubash");
    let normalize = |text: &str| {
        text.replace(&dir.to_string_lossy().into_owned(), "WORK")
            .replace(&script_path.to_string_lossy().into_owned(), "SCRIPT")
    };
    (
        normalize(&String::from_utf8_lossy(&output.stdout)),
        normalize(&String::from_utf8_lossy(&output.stderr)),
    )
}

#[test]
fn issue2434_braced_faces_read_empty_after_unset() {
    let dir = scratch("faces");
    let (out, err) = run_script(
        &dir,
        r#"cd "$1/a" || exit 1
pushd "$1/b" >/dev/null
unset DIRSTACK
echo "e0=[${DIRSTACK[0]}] count=${#DIRSTACK[@]} bare=[$DIRSTACK] all=[${DIRSTACK[@]}]"
"#,
    );
    assert_eq!(err, "");
    assert_eq!(out, "e0=[] count=0 bare=[] all=[]\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2434_pushd_does_not_resurrect_and_dirs_still_works() {
    let dir = scratch("pushd");
    let (out, err) = run_script(
        &dir,
        r#"cd "$1/a" || exit 1
pushd "$1/b" >/dev/null
unset DIRSTACK
pushd "$1/a" >/dev/null
echo "count=${#DIRSTACK[@]} all=[${DIRSTACK[@]}]"
dirs
"#,
    );
    assert_eq!(err, "");
    assert_eq!(
        out,
        "count=0 all=[]\nWORK/a WORK/b WORK/a\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2434_later_binds_create_a_plain_variable() {
    let dir = scratch("rebind");
    let (out, err) = run_script(
        &dir,
        r#"unset DIRSTACK
DIRSTACK[0]=ZZ
echo "e0=${DIRSTACK[0]} count=${#DIRSTACK[@]}"
DIRSTACK=(p q)
pushd "$1/a" >/dev/null
echo "after pushd: all=${DIRSTACK[@]} count=${#DIRSTACK[@]}"
declare DIRSTACK=(x y)
cd "$1/b" || exit 1
echo "after declare+cd: all=${DIRSTACK[@]}"
"#,
    );
    assert_eq!(err, "");
    assert_eq!(
        out,
        "e0=ZZ count=1\nafter pushd: all=p q count=2\nafter declare+cd: all=x y\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2434_unset_inside_a_function_unbinds_the_global() {
    let dir = scratch("func");
    let (out, err) = run_script(
        &dir,
        r#"pushd "$1/a" >/dev/null
f() { unset DIRSTACK; }
f
echo "after f: count=${#DIRSTACK[@]} e0=[${DIRSTACK[0]}]"
"#,
    );
    assert_eq!(err, "");
    assert_eq!(out, "after f: count=0 e0=[]\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2434_unsetting_a_local_shadow_leaves_the_global_dynamic() {
    let dir = scratch("shadow");
    let (out, err) = run_script(
        &dir,
        r#"pushd "$1/a" >/dev/null
f() { local DIRSTACK=(L1 L2); unset DIRSTACK; echo "in: count=${#DIRSTACK[@]}"; }
f
echo "after f: count=${#DIRSTACK[@]} top=[${DIRSTACK[0]}]"
"#,
    );
    assert_eq!(err, "");
    assert_eq!(out, "in: count=0\nafter f: count=2 top=[WORK/a]\n");
    let _ = fs::remove_dir_all(&dir);
}
