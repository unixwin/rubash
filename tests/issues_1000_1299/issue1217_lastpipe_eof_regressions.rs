//! rubash#1217 — lastpipe: EOF `read` gets SIGHUP in GNU (rc=129);
//! niu returns rc=1.
//!
//! On the glibc GNU baseline the filed SIGHUP form does NOT occur:
//! with lastpipe on and job control off, an EOF read as the last
//! pipeline stage simply returns 1 in the current shell, and a fed
//! read returns 0 with the value visible afterwards — GNU bash
//! 5.3.20 and the fixed engine agree item for item (the reported
//! rc=129 was specific to the reporter's device session). These pins
//! hold the glibc-baseline agreement. (The `read -t 0` sub-form from
//! the device matrix is a documented residual — GNU's -t 0 polls its
//! internal input buffer; the minimal glibc form of it agrees with
//! the engine (both rc=0), so no separate pin is written for it.)
//!
//! Expectations verified against GNU bash 5.3.20 with SCRIPT FILES.

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1217-{tag}-{}", std::process::id()));
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
fn lastpipe_eof_read_returns_1_in_current_shell() {
    let (stdout, stderr, code) = rubash_file(
        "eof",
        "shopt -s lastpipe\n: | read x; echo \"rc=$?\"\necho hi | read y; echo \"rc=$? y=$y\"\n",
    );
    assert_eq!(stdout, "rc=1\nrc=0 y=hi\n");
    assert_eq!(stderr, "", "no job-control notification may appear");
    assert_eq!(code, Some(0));
}
