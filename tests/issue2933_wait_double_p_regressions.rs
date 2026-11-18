//! rubash#2933 — `wait` with two `-p` options validates the first one;
//! GNU's getopt loop (builtins/wait.def:125-157) overwrites the name on
//! each `-p` and validates after the loop, so the LAST `-p` decides.
//!
//! Expectations cross-checked against GNU bash 5.3.20 (Stage-2 ④
//! Phase 1 migration study).

use std::path::PathBuf;
use std::process::Command;

struct Run {
    status: i32,
    stderr: String,
    dir: PathBuf,
}

fn run_script(body: &str) -> Run {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("issue2933_{}_{}", std::process::id(), seq));
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
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        dir,
    }
}

#[test]
fn wait_double_p_validates_the_last_name() {
    let run = run_script("wait -p A -p 1bad\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert!(
        run.stderr.contains("`1bad': not a valid identifier"),
        "stderr: {}",
        run.stderr
    );
    let _ = run.dir;
}

#[test]
fn wait_double_p_last_valid_name_passes() {
    let run = run_script("wait -p 1bad -p A\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    let _ = run.dir;
}

#[test]
fn wait_double_p_both_valid_passes() {
    let run = run_script("wait -p A -p B\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    let _ = run.dir;
}
