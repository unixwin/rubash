//! rubash#2931 — `jobs -x` mishandles other options: GNU's getopt loop
//! (builtins/jobs.def:93-124) reports `no other options allowed with
//! \`-x'` (EXECUTION_FAILURE, no usage line) when a form-setting option
//! (`-l`/`-p`/`-n`) precedes `-x`, and keeps scanning after `-x`, so
//! `-x -l echo hi` runs `echo hi`. rubash's `-x` arm took every
//! following word as the command and never checked the form.
//!
//! Expectations cross-checked against GNU bash 5.3.20 (Stage-2 ④
//! Phase 1 E1 differential, 261 cells).

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
    let dir = std::env::temp_dir().join(format!("issue2931_{}_{}", std::process::id(), seq));
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

const NO_OTHER_OPTIONS: &str = "jobs: no other options allowed with `-x'";

#[test]
fn jobs_l_before_x_is_an_error() {
    let run = run_script("jobs -lx\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert!(run.stdout.is_empty(), "stdout: {}", run.stdout);
    assert!(
        run.stderr.contains(NO_OTHER_OPTIONS),
        "stderr: {}",
        run.stderr
    );
    assert!(
        !run.stderr.contains("usage:"),
        "the -x form error carries no usage line: {}",
        run.stderr
    );
    let _ = run.dir;
}

#[test]
fn jobs_l_word_before_x_command_is_an_error() {
    let run = run_script("jobs -l -x echo hi\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert!(run.stdout.is_empty(), "stdout must not print hi: {}", run.stdout);
    assert!(
        run.stderr.contains(NO_OTHER_OPTIONS),
        "stderr: {}",
        run.stderr
    );
    let _ = run.dir;
}

#[test]
fn jobs_p_before_x_is_an_error() {
    let run = run_script("jobs -px\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert!(
        run.stderr.contains(NO_OTHER_OPTIONS),
        "stderr: {}",
        run.stderr
    );
    let _ = run.dir;
}

#[test]
fn jobs_x_then_l_runs_the_command() {
    let run = run_script("jobs -x -l echo hi\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "hi\n", "stdout: {}", run.stdout);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    let _ = run.dir;
}

#[test]
fn jobs_x_then_p_runs_the_command() {
    let run = run_script("jobs -x -p echo hi\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "hi\n", "stdout: {}", run.stdout);
    let _ = run.dir;
}

#[test]
fn jobs_x_then_invalid_option_is_usage() {
    let run = run_script("jobs -x -z echo hi\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(
        run.stderr.contains("jobs: -z: invalid option"),
        "stderr: {}",
        run.stderr
    );
    assert!(run.stderr.contains("usage:"), "stderr: {}", run.stderr);
    let _ = run.dir;
}

#[test]
fn jobs_xl_cluster_runs_the_command() {
    // 'x' first: the form check passes, the 'l' after it only sets the
    // (irrelevant for execution) form — the command still runs.
    let run = run_script("jobs -xl echo hi\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "hi\n", "stdout: {}", run.stdout);
    let _ = run.dir;
}

#[test]
fn jobs_r_before_x_does_not_touch_the_form() {
    // 'r'/'s' set the state, not the form, so they never trip the check.
    let run = run_script("jobs -rx\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.is_empty(), "stdout: {}", run.stdout);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    let _ = run.dir;
}

#[test]
fn jobs_x_alone_runs_the_empty_command() {
    let run = run_script("jobs -x\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.is_empty(), "stdout: {}", run.stdout);
    let _ = run.dir;
}
