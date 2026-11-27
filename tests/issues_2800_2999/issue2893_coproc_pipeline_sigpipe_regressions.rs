//! rubash#2893 — a coproc used as a pipeline element printed a
//! spurious "Broken pipe" diagnostic. The pipeline stage spawner
//! blocks SIGPIPE on the executor thread while it works; the
//! coproc child (a respawned shell) was spawned inside that window
//! and inherited the blocked mask across execve, so its write to a
//! reader-less pipe returned EPIPE (and the child shell printed
//! the diagnostic) instead of the child dying silently of SIGPIPE
//! as GNU's forked element does (SIGPIPE unblocked, SIG_DFL).
//! The coproc spawn now unblocks SIGPIPE in the child pre-exec.
//!
//! Expectations cross-checked against GNU bash 5.3.20 (script-file
//! probes in the r75 hunt; pre-fix rubash printed the diagnostic
//! in 12/12 runs of both shapes, post-fix in 0/12).

use std::path::PathBuf;
use std::process::Command;

struct Run {
    status: i32,
    stdout: String,
    stderr: String,
    dir: PathBuf,
}

fn run_script(body: &str) -> Run {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("issue2893_{}_{}", std::process::id(), seq));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    std::fs::write(dir.join("case.sh"), body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    Run {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        dir,
    }
}

#[test]
fn coproc_pipeline_head_prints_no_broken_pipe() {
    // GNU shape: the element's coproc child writes to the coproc
    // pipe owned by the forked element (whose registration is
    // discarded), so `cat` receives nothing and the child's output
    // is simply lost — the only observable contract is the ABSENCE
    // of the spurious diagnostic (the child dies of SIGPIPE
    // silently when its reader goes away).
    let run = run_script("coproc { echo hi; } | cat\necho done\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "done\n", "stdout: {}", run.stdout);
    assert!(
        !run.stderr.contains("Broken pipe"),
        "stderr: {}",
        run.stderr
    );
    let _ = run.dir;
}

#[test]
fn coproc_pipeline_middle_prints_no_broken_pipe() {
    let run = run_script("printf 'x\\n' | coproc { echo alive; } | cat\necho done\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "done\n", "stdout: {}", run.stdout);
    assert!(
        !run.stderr.contains("Broken pipe"),
        "stderr: {}",
        run.stderr
    );
}
