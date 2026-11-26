//! rubash#2898 — `wait` with a numeric operand above INT_MAX was
//! treated as a valid (unknown) pid and reported 127 "not a
//! child". GNU's wait.def classifies a digit-leading word as a pid
//! only when valid_number(w) && pid_value == (pid_t)pid_value
//! (builtins/wait.def:272-291; pid_t is a 32-bit int): anything
//! larger is not a pid at all — sh_badpid ("wait: `N': not a pid or
//! valid job spec") and an immediate EXECUTION_FAILURE (1), without
//! looking at the remaining operands. An in-range operand keeps
//! the ordinary path and its diagnostics print the PARSED value
//! (jobs.c:2753: `wait 007` reports "pid 7"). Under `wait -n` an
//! out-of-range operand additionally earns the "job specification
//! requires leading `%'" warning on the way to the badpid report
//! (rc stays 127 on that path).
//!
//! Expectations cross-checked against GNU bash 5.3.20 (script-file
//! probes in the r75 hunt).

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
    let dir = std::env::temp_dir().join(format!("issue2898_{}_{}", std::process::id(), seq));
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
fn wait_pid_above_int_max_is_badpid_with_status_1() {
    let run = run_script(
        "wait 2147483648; echo \"rc:$?\"\nwait 99999999999999999999; echo \"rc:$?\"\nwait 2147483647; echo \"rc:$?\"\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "rc:1\nrc:1\nrc:127\n", "stdout: {}", run.stdout);
    assert_eq!(
        run.stderr.matches("not a pid or valid job spec").count(),
        2,
        "stderr: {}",
        run.stderr
    );
    assert!(
        run.stderr
            .contains("pid 2147483647 is not a child of this shell"),
        "stderr: {}",
        run.stderr
    );
    let _ = run.dir;
}

#[test]
fn wait_leading_zero_pid_prints_the_parsed_value() {
    let run = run_script("wait 007; echo \"rc:$?\"\nwait 00; echo \"rc:$?\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "rc:127\nrc:127\n", "stdout: {}", run.stdout);
    assert!(
        run.stderr.contains("pid 7 is not a child of this shell"),
        "stderr: {}",
        run.stderr
    );
    assert!(
        run.stderr.contains("pid 0 is not a child of this shell"),
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn wait_badpid_operand_short_circuits_the_list() {
    let run = run_script(
        "wait 2147483648 007; echo \"rc:$?\"\nwait 007 2147483648; echo \"rc:$?\"\nwait -n 2147483648; echo \"rc:$?\"\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "rc:1\nrc:1\nrc:127\n", "stdout: {}", run.stdout);
    // The -n path warns about the missing `%` before the badpid
    // report; both plain paths report the bad operand directly.
    assert!(
        run.stderr.contains("job specification requires leading"),
        "stderr: {}",
        run.stderr
    );
    assert_eq!(
        run.stderr.matches("not a pid or valid job spec").count(),
        3,
        "stderr: {}",
        run.stderr
    );
}
