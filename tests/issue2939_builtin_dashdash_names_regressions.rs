//! rubash#2939 — `builtin` treats words after `--` as options instead of
//! names (Stage-2 ④ Phase 2 E1).
//!
//! GNU anchor: `builtins/builtin.def:58` (`no_options`) +
//! `builtins/bashgetopt.c` — `--` is consumed and ends the scan; the
//! following words are operands: the names of builtins to run.
//!
//! Expectations cross-checked against GNU bash 5.3.20 (E1 differential,
//! 532 cells, master `854eed80`).

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
    let dir = std::env::temp_dir().join(format!("issue2939_{}_{}", std::process::id(), seq));
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
fn dashdash_dash_x_is_a_name_not_an_option() {
    // GNU: `-x: not a shell builtin`, rc 1, no usage line.
    let run = run_script("builtin -- -x\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert!(run.stdout.is_empty(), "stdout: {}", run.stdout);
    assert!(
        run.stderr.contains("builtin: -x: not a shell builtin"),
        "stderr: {}",
        run.stderr
    );
    assert!(!run.stderr.contains("usage:"), "stderr: {}", run.stderr);
    let _ = run.dir;
}

#[test]
fn dashdash_help_is_a_name_not_an_option() {
    // GNU: `--help: not a shell builtin`, rc 1, no usage line.
    let run = run_script("builtin -- --help\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert!(run.stdout.is_empty(), "stdout: {}", run.stdout);
    assert!(
        run.stderr.contains("builtin: --help: not a shell builtin"),
        "stderr: {}",
        run.stderr
    );
    assert!(!run.stderr.contains("usage:"), "stderr: {}", run.stderr);
    let _ = run.dir;
}

#[test]
fn dashdash_name_still_runs_the_builtin() {
    // Positive control: a real builtin name after `--` runs with its args.
    let run = run_script("builtin -- echo hi\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "hi\n", "stdout: {}", run.stdout);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    let _ = run.dir;
}

#[test]
fn dashdash_unknown_name_reports_not_a_builtin() {
    // Positive control: an unknown name after `--` is the same
    // not-a-builtin error as without `--`.
    let run = run_script("builtin -- nosuchbuiltin123\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert!(
        run.stderr
            .contains("builtin: nosuchbuiltin123: not a shell builtin"),
        "stderr: {}",
        run.stderr
    );
    let _ = run.dir;
}
