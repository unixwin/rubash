//! rubash#2585 — `expansion_probe_active` was a plain bool armed and
//! disarmed by an unconditional set(true)/set(false) pair around the
//! #809 async-gate word-expansion probe (compound_exec.rs). A nested
//! probe on the same executor (a comsub/async gate inside the probed
//! expansion arming its own) ran its set(false) and silently disarmed
//! the OUTER probe, re-admitting exactly what the probe contract
//! forbids: the doubled `no match` failglob diagnostic (parent probe +
//! wrapper child) and the armed evalerror discard that swallows the
//! rest of the launch line. The flag is now armed through an RAII
//! guard restoring the prior value (the SpeculativeExpansionGuard
//! shape, pipeline_exec.rs); the guard's save/restore semantics are
//! pinned by the in-crate unit tests
//! `expansion_probe_guard_tests::issue2585_*` (cargo test --lib).
//!
//! Oracle expectation (GNU bash 5.3.20): GNU expands the async child's
//! words inside the forked child, so the parent never reports — the
//! diagnostic appears exactly once and the rest of the line runs.

use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn run_script(script: &str) -> (String, String, Option<i32>) {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("rubash-2585-{}-{n}", std::process::id()));
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
fn issue_2585_async_failglob_reports_once_and_line_continues() {
    let (stdout, stderr, _code) =
        run_script("shopt -s failglob\necho /no/such/*.zzz & echo BG_after\nwait\necho done\n");
    assert!(
        stdout.contains("BG_after"),
        "the launch line's remainder must survive the probe, stdout: {stdout}, stderr: {stderr}"
    );
    assert!(stdout.contains("done"), "stdout: {stdout}");
    assert_eq!(
        stderr.matches("no match").count(),
        1,
        "failglob diagnostic must appear exactly once, stderr: {stderr}"
    );
}
