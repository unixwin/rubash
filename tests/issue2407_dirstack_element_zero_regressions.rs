//! rubash#2407 — `DIRSTACK[0]=x` must be a no-op.
//!
//! GNU routes every DIRSTACK element assignment through
//! variables.c:1612 assign_dirstack into builtins/pushd.def:657
//! set_dirstack_element, whose first test (pushd.def:662) is
//! `if (ind == 0 || ...) return;`: slot 0 is not stored state, it is
//! the working directory itself, produced at read time by
//! get_directory_stack (pushd.def:669-682, appending
//! get_working_directory after the pushed entries). rubash serializes
//! slot 0 inside its stack carrier, and its element-assignment door
//! (array_assignment_exec -> set_stack_value) overwrote it, so
//! `DIRSTACK[0]=/bogus` desynced every read face (dirs,
//! ${DIRSTACK[0]}) from $PWD until the next cd/pushd refresh.
//!
//! Expectations byte-verified against the GNU bash 5.3.20 oracle with
//! script-file probes (the issue's Regression seed); the scratch path
//! is normalized to WORK, mirroring tests/issue2391_*.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2407-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    for sub in ["a", "b"] {
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
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .output()
        .expect("run rubash");
    assert!(
        output.status.success(),
        "probe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).replace(&dir.to_string_lossy().into_owned(), "WORK")
}

#[test]
fn issue2407_dirstack0_assignment_is_noop_after_pushd() {
    let dir = scratch("seed");
    let out = run_script(
        &dir,
        r#"cd "$1/a" || exit 1
pushd "$1/b" >/dev/null
DIRSTACK[0]=/bogus
echo "dirs: $(dirs)"
echo "top: ${DIRSTACK[0]}"
echo "pwd: $PWD"
"#,
    );
    assert_eq!(out, "dirs: WORK/b WORK/a\ntop: WORK/b\npwd: WORK/b\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2407_dirstack0_assignment_is_noop_without_stack() {
    // No pushd yet: no stored stack exists, and the no-op must not
    // materialize one carrying the bogus top either.
    let dir = scratch("nostack");
    let out = run_script(
        &dir,
        r#"cd "$1/a" || exit 1
DIRSTACK[0]=/bogus
echo "dirs: $(dirs)"
echo "top: ${DIRSTACK[0]}"
"#,
    );
    assert_eq!(out, "dirs: WORK/a\ntop: WORK/a\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn issue2407_dirstack_nonzero_assignment_still_works() {
    // The guard is index 0 only: GNU set_dirstack_element stores
    // every in-range index >= 1 (pushd.def:663-665).
    let dir = scratch("nonzero");
    let out = run_script(
        &dir,
        r#"cd "$1/a" || exit 1
pushd "$1/b" >/dev/null
DIRSTACK[1]=/two
echo "dirs: $(dirs)"
echo "ds1: ${DIRSTACK[1]}"
"#,
    );
    assert_eq!(out, "dirs: WORK/b /two\nds1: /two\n");
    let _ = fs::remove_dir_all(&dir);
}
