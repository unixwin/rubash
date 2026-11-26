//! rubash#200 — `set -u` diagnostics must name the variable that is
//! actually unbound.
//!
//! GNU subst.c: the unbound-variable error reports the EXPANSION's
//! variable — a plain unset name reports itself (`UNSET`), and an
//! out-of-range array element in `${#arr[i]}` reports the element
//! (`narray[4]`) — never some other variable that happens to be in
//! scope. The filed bug named an unrelated variable.
//!
//! Expectations verified against GNU bash 5.3.20 with SCRIPT FILES.

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash200-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn rubash_file(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = tempdir(tag);
    std::fs::write(dir.join("probe.sh"), script).expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("probe.sh")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn unbound_error_names_the_expanded_variable() {
    let (stdout, stderr, code) = rubash_file("plain", "set -u; echo $UNSET\n");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("UNSET: unbound variable"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(1));
}

#[test]
fn unbound_error_names_the_array_element() {
    // The filed probe never assigns the array: with the array itself
    // unset, the element reference is the unbound name. (A set-but-
    // short array is different — ${#arr[4]} is then just 0 in GNU
    // too.)
    let (stdout, stderr, code) =
        rubash_file("elem", "set -u; ( echo ${#narray[4]} ); echo \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("narray[4]: unbound variable"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0), "the subshell absorbs the fatal error");
}
