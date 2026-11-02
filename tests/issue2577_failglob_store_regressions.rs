//! rubash#2577 — `failglob` is a shopt option (GNU builtins/shopt.def:219,
//! `{ "failglob", &fail_glob_expansion, NULL }` in shopt_vars), so every
//! reader must consult the SHOPT store. Two readers consulted the
//! `set -o` store instead (`shell_option_enabled(..., "failglob")` reads
//! `__RUBASH_SETOPT_failglob`, which shopt never writes): the concurrent
//! pipeline admission probe (parameter_errors.rs) and the direct-coproc
//! planner (compound_exec.rs). With the bug, a pipeline member carrying an
//! unmatched glob under `shopt -s failglob` was admitted to the concurrent
//! fast path, which could duplicate or drop the `no match:` diagnostic.
//!
//! Expectations measured against GNU bash 5.3.20 (oracle) in an empty
//! scratch directory: the diagnostic appears exactly once, the failing
//! element reports status 1 in PIPESTATUS, and the script continues.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn run_script(script: &str) -> (String, String, Option<i32>) {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("rubash-2577-{}-{n}", std::process::id()));
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
fn issue_2577_failglob_pipeline_member_reports_once() {
    let (stdout, stderr, code) = run_script(
        "shopt -s failglob\necho nope* | cat\necho \"st=${PIPESTATUS[0]}\"\necho after\n",
    );
    assert_eq!(stdout, "st=1\nafter\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr.matches("no match: nope*").count(),
        1,
        "diagnostic must appear exactly once, stderr: {stderr}"
    );
}

#[test]
fn issue_2577_failglob_off_pipeline_member_passes_word_through() {
    // Control: without failglob the unmatched word reaches the pipe
    // literally (the plain glob.rs SHOPT-store reader already worked).
    let (stdout, stderr, code) = run_script("echo nope* | cat\necho \"st=${PIPESTATUS[0]}\"\n");
    assert_eq!(stdout, "nope*\nst=0\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert!(stderr.is_empty(), "stderr: {stderr}");
}
