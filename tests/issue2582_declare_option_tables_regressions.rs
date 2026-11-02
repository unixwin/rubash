//! rubash#2582 — three hand-copied declare-family option-letter
//! tables disagreed in both directions. GNU has ONE table:
//! declare.def:145 `#define DECLARE_OPTS "+acfgilnprtuxAFGI"`.
//! - `local_helpers::validate_local_options` was missing `c` (and
//!   `G`), so `local -c x=hello` → "invalid option" rc=2, while GNU
//!   applies capcase.
//! - `builtins::declare`'s scanner was missing `f`/`F`, so
//!   `local -f g` passed the local validator then died in the declare
//!   scanner — while GNU routes local's -f/-F to the same
//!   function-listing path as declare's (local is declare_internal in
//!   a local context).
//!
//! Oracle (GNU bash 5.3.20):
//!   f() { local -c x=hello; echo "x=$x"; }  →  x=Hello
//!   f() { local -f g; }  →  prints g's definition, rc 0
//!   f() { local -F g; }  →  prints "g", rc 0

use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn run_script(script: &str) -> (String, String, Option<i32>) {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("rubash-2582-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(path.to_str().expect("utf8 path"))
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let out = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    out
}

#[test]
fn issue_2582_local_capcase_accepted() {
    let (stdout, stderr, _code) = run_script("f() { local -c x=hello; echo \"x=$x\"; }\nf\n");
    assert_eq!(stdout, "x=Hello\n", "stderr: {stderr}");
    assert!(!stderr.contains("invalid option"), "stderr: {stderr}");
}

#[test]
fn issue_2582_local_function_listing_accepted() {
    let (stdout, stderr, _code) =
        run_script("g() { echo in-g; }\nf() { local -f g; echo \"rc_f=$?\"; local -F g; echo \"rc_F=$?\"; }\nf\n");
    assert!(
        stdout.contains("in-g"),
        "local -f must list the function body, stdout: {stdout}, stderr: {stderr}"
    );
    assert!(stdout.contains("rc_f=0"), "stdout: {stdout}");
    assert!(stdout.contains("rc_F=0"), "stdout: {stdout}");
    assert!(!stderr.contains("invalid option"), "stderr: {stderr}");
}

#[test]
fn issue_2582_local_invalid_option_still_rejected() {
    // Control: a letter outside DECLARE_OPTS keeps the local usage error.
    let (stdout, stderr, _code) = run_script("f() { local -Z x; echo \"rc=$?\"; }\nf\n");
    assert!(
        stdout.contains("rc=2"),
        "stdout: {stdout}, stderr: {stderr}"
    );
    assert!(stderr.contains("invalid option"), "stderr: {stderr}");
}

#[test]
fn issue_2582_declare_capcase_control() {
    // Control: plain declare -c already worked and must not regress.
    let (stdout, stderr, _code) = run_script("declare -c x=hello\necho \"x=$x\"\n");
    assert_eq!(stdout, "x=Hello\n", "stderr: {stderr}");
}
