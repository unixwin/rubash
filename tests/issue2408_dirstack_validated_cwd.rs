//! rubash#2408 — a forged `$PWD` must not leak into the directory
//! stack's element 0, and `DIRSTACK=(...)` must route through the
//! stack setter instead of binding a phantom array cell.
//!
//! GNU never stores the current directory in the stack list:
//! get_directory_stack (builtins/pushd.def:669) appends
//! get_working_directory — the cd.def-validated logical cwd, which
//! honors $PWD only while it still names the working directory —
//! after the pushed entries. So after `PWD=/elsewhere` with no stack
//! operations at all, `dirs`, `dirs -v`, `dirs -p`, ${DIRSTACK[0]},
//! ${DIRSTACK[@]} and `declare -p DIRSTACK` all still show the REAL
//! cwd. The one face that follows the forged variable is the tilde
//! dirstack lookup: get_dirstack_from_string (pushd.def:620) binds an
//! index landing on element 0 to get_string_value("PWD") verbatim, so
//! `~+0` (and `~+`) expand to the forged path — GNU splits there on
//! purpose, and these pins assert the split, not uniformity.
//!
//! Compound assignment routes every element through the same setter
//! as element assignment (variables.c:1612 assign_dirstack ->
//! pushd.def set_dirstack_element): element 0 is ignored, in-bounds
//! elements replace, out-of-range elements drop silently, and the
//! list is never cleared or grown — `DIRSTACK=(...)` on an empty
//! stack is a no-op, `DIRSTACK=()` never clears, and a scalar bind is
//! an element-0 bind (also a no-op). Before this fix the executor had
//! no DIRSTACK door for whole-variable assignment at all: the value
//! landed in a parallel typed cell, `dirs` kept showing the carrier
//! view while ${DIRSTACK[1]} read the phantom, and `declare -p
//! DIRSTACK` panicked on the mirror disagreement.
//!
//! Expectations byte-verified against the GNU bash 5.3.20 oracle with
//! script-file probes; the scratch path is normalized to WORK.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2408-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    for sub in ["real", "other", "elsewhere"] {
        fs::create_dir_all(dir.join(sub)).expect("create scratch subdir");
    }
    dir
}

/// Run `script` as a script FILE whose $1 is the scratch dir; return
/// (stdout, stderr, success) with the scratch path normalized to WORK.
fn run_script(dir: &Path, script: &str) -> (String, String, bool) {
    let script_path = dir.join("probe.sh");
    fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(RUBASH)
        .arg(&script_path)
        .arg(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let normalize = |bytes: &[u8]| {
        String::from_utf8_lossy(bytes).replace(&dir.to_string_lossy().into_owned(), "WORK")
    };
    (
        normalize(&output.stdout),
        normalize(&output.stderr),
        output.status.success(),
    )
}

fn run_script_ok(dir: &Path, script: &str) -> String {
    let (stdout, stderr, success) = run_script(dir, script);
    assert!(success, "probe failed: {stderr}");
    stdout
}

#[test]
fn issue2408_forged_pwd_no_stack_ops_shows_real_cwd() {
    let dir = scratch("leak");
    let out = run_script_ok(
        &dir,
        r#"cd "$1/real" || exit 1
PWD="$1/elsewhere"
echo "dirs: $(dirs)"
dirs -v
dirs -p
echo "ds0: ${DIRSTACK[0]}"
echo "all: ${DIRSTACK[@]}"
echo "n: ${#DIRSTACK[@]}"
declare -p DIRSTACK
echo "tilde0:" ~+0
echo "tildep:" ~+
echo "pwdvar: $PWD"
"#,
    );
    assert_eq!(
        out,
        "dirs: WORK/real\n\
         \x200  WORK/real\n\
         WORK/real\n\
         ds0: WORK/real\n\
         all: WORK/real\n\
         n: 1\n\
         declare -a DIRSTACK=([0]=\"WORK/real\")\n\
         tilde0: WORK/elsewhere\n\
         tildep: WORK/elsewhere\n\
         pwdvar: WORK/elsewhere\n"
    );
}

#[test]
fn issue2408_forged_pwd_after_pushd_control() {
    let dir = scratch("control");
    let out = run_script_ok(
        &dir,
        r#"cd "$1/real" || exit 1
pushd "$1/other" >/dev/null
PWD="$1/elsewhere"
echo "dirs: $(dirs)"
echo "ds0: ${DIRSTACK[0]}"
echo "tilde0:" ~+0
echo "tilde1:" ~+1
echo "tildem2:" ~-2
echo "tildem0:" ~-0
echo "tildem1:" ~-1
"#,
    );
    // With a carrier the stack faces were already correct; the tilde
    // split is GNU's: ~+0 and the far-end ~-1 bind the forged $PWD,
    // ~-2 is out of range and keeps the word verbatim.
    assert_eq!(
        out,
        "dirs: WORK/other WORK/real\n\
         ds0: WORK/other\n\
         tilde0: WORK/elsewhere\n\
         tilde1: WORK/real\n\
         tildem2: ~-2\n\
         tildem0: WORK/real\n\
         tildem1: WORK/elsewhere\n"
    );
}

#[test]
fn issue2408_compound_assignment_uses_stack_setter() {
    let dir = scratch("compound");
    let out = run_script_ok(
        &dir,
        r#"cd "$1/real" || exit 1
DIRSTACK=(/e0 /e1 /e2)
echo "empty-compound: $(dirs)"
pushd "$1/other" >/dev/null
pushd "$1/elsewhere" >/dev/null
DIRSTACK=(/e0 /e1 /e2)
echo "after-compound: $(dirs)"
DIRSTACK=()
echo "after-empty: $(dirs)"
DIRSTACK=(/only0)
echo "after-only0: $(dirs)"
DIRSTACK=([2]=/s2 [0]=/s0)
echo "after-sub: $(dirs)"
DIRSTACK=([1+1]=/s3)
echo "after-arith: $(dirs)"
DIRSTACK+=(/app)
echo "after-append: $(dirs)"
DIRSTACK+=([1]=/w)
echo "after-append-sub: $(dirs)"
DIRSTACK=/scalar
echo "rc-scalar=$? dirs: $(dirs)"
declare -p DIRSTACK
echo "ds1: ${DIRSTACK[1]}"
"#,
    );
    assert_eq!(
        out,
        "empty-compound: WORK/real\n\
         after-compound: WORK/elsewhere /e1 /e2\n\
         after-empty: WORK/elsewhere /e1 /e2\n\
         after-only0: WORK/elsewhere /e1 /e2\n\
         after-sub: WORK/elsewhere /e1 /s2\n\
         after-arith: WORK/elsewhere /e1 /s3\n\
         after-append: WORK/elsewhere /e1 /s3\n\
         after-append-sub: WORK/elsewhere /w /s3\n\
         rc-scalar=0 dirs: WORK/elsewhere /w /s3\n\
         declare -a DIRSTACK=([0]=\"WORK/elsewhere\" [1]=\"/w\" [2]=\"/s3\")\n\
         ds1: /w\n"
    );
}

#[test]
fn issue2408_compound_bad_subscript_reports_and_keeps_stack() {
    let dir = scratch("badsub");
    let (out, err, success) = run_script(
        &dir,
        r#"cd "$1/real" || exit 1
pushd "$1/other" >/dev/null
( DIRSTACK=([-1]=/neg) )
echo "rc=$?"
echo "dirs: $(dirs)"
"#,
    );
    assert!(success, "probe failed: {err}");
    assert_eq!(out, "rc=1\ndirs: WORK/other WORK/real\n");
    assert!(
        err.contains("[-1]=/neg: bad array subscript"),
        "expected a bad-subscript diagnostic, got: {err}"
    );
}
