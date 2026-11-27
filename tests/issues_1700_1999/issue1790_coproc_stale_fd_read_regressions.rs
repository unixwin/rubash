//! rubash#1790 — second read on a stale coproc fd: adjudication pin.
//!
//! The issue (measured on the merged #1694 tree) claimed the second
//! read prints no invalid-fd diagnostic and empties the target
//! variable. Re-measured on this cluster's baseline master
//! (f78396f3) with the deterministic form of the shape — fd number
//! saved before the child ends, a settle delay so the death is
//! processed — BOTH claims fail to reproduce: the diagnostic
//! (`read: 63: invalid file descriptor: Bad file descriptor`) is
//! printed and the variable keeps its value, byte-identical to GNU
//! bash 5.3.20, 3/3 on both sides. The only divergence in the shape
//! was the FIRST read's exit code, which is rubash#1623 (the
//! EOF-after-full-line misreport) and is fixed by that issue's
//! change; the un-saved-fd form of the shape is racy in GNU itself
//! (3 runs: the family may retire before the first read, failing it
//! with `invalid file descriptor specification`).
//!
//! This pin locks the GNU behavior in so the adjudication stays
//! verifiable: diagnostic present, variable preserved, rc 1.

use std::process::{Command, Stdio};

fn rubash_script(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-1790-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir(&dir).expect("create scratch dir");
    let path = dir.join("s.sh");
    std::fs::write(&path, script).expect("seed script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let result = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn coproc_second_read_on_stale_fd_diagnoses_and_preserves_variable() {
    let (stdout, stderr, code) = rubash_script(
        "coproc MYC { echo hi; }\n\
         fd=${MYC[0]}\n\
         read -u $fd v; echo \"r1 rc=$? v=[$v]\"\n\
         sleep 0.3\n\
         v=SENTINEL\n\
         read -u $fd v; echo \"r2 rc=$? v=[$v]\"\n",
    );
    assert_eq!(
        stdout, "r1 rc=0 v=[hi]\nr2 rc=1 v=[SENTINEL]\n",
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("read: 63: invalid file descriptor: Bad file descriptor"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}
