//! rubash#2897 — `wait` inside a parenthesized subshell reaped
//! the PARENT shell's jobs: the subshell shares the parent's
//! executor in rubash, and its job table / background-child
//! handles were fully exposed to the body. GNU runs the subshell
//! through execute_in_subshell -> without_job_control (jobs.c:5356)
//! -> delete_all_jobs(0) (jobs.c:5065) plus bgp_clear: the subshell
//! starts with an EMPTY job list, so `( wait $p )` reports "not a
//! child of this shell" (127) and the parent's own wait afterwards
//! still returns the job's real status. The paren-subshell entries
//! now swap the process state out for the body's duration and
//! clear the job table (the #2768 reports share this root: the
//! subshell also inherited the table for `jobs`/`disown`).
//!
//! Expectations cross-checked against GNU bash 5.3.20 (script-file
//! probes s97a-s97g in the r75 hunt).

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
    let dir = std::env::temp_dir().join(format!("issue2897_{}_{}", std::process::id(), seq));
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
fn subshell_wait_cannot_reap_the_parents_job() {
    let run =
        run_script("sleep 0.3 & p=$!\n( wait $p; echo \"sub:$?\" )\nwait $p; echo \"outer:$?\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "sub:127\nouter:0\n", "stdout: {}", run.stdout);
    assert!(
        run.stderr.contains("not a child of this shell"),
        "stderr: {}",
        run.stderr
    );
    let _ = run.dir;
}

#[test]
fn subshell_bare_wait_leaves_the_parents_job_alone() {
    let run = run_script(
        "sleep 0.3 & p=$!\n( wait; echo \"bare-sub:$?\" )\nwait $p; echo \"outer:$?\"\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout, "bare-sub:0\nouter:0\n",
        "stdout: {}",
        run.stdout
    );
}

#[test]
fn subshell_wait_on_an_already_dead_parent_job_is_still_not_a_child() {
    let run = run_script(
        "sleep 0.1 & p=$!\nsleep 0.3\n( wait $p; echo \"sub:$?\" )\nwait $p; echo \"outer:$?\"\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "sub:127\nouter:0\n", "stdout: {}", run.stdout);
}

#[test]
fn subshell_sees_an_empty_job_table() {
    let run = run_script(
        "sleep 5 & p=$!\n( jobs; echo \"jobs-rc:$?\" )\n( kill %1 2>&1; echo \"kill-rc:$?\" )\njobs\nkill $p 2>/dev/null; wait $p 2>/dev/null; echo \"cleanup:$?\"\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    // The subshell's `jobs` prints nothing; the parent's `jobs`
    // still lists the job; the subshell's kill %1 fails.
    assert!(run.stdout.contains("jobs-rc:0"), "stdout: {}", run.stdout);
    assert!(
        run.stdout.contains("kill-rc:1") || run.stdout.contains("kill-rc:2"),
        "stdout: {}",
        run.stdout
    );
    // The kill's diagnostic is merged into the subshell's stdout
    // by the case's own 2>&1.
    assert!(run.stdout.contains("no such job"), "stdout: {}", run.stdout);
    assert!(
        run.stdout.matches("sleep 5").count() == 1,
        "only the parent's jobs lists the job; stdout: {}",
        run.stdout
    );
}

#[test]
fn subshells_own_jobs_still_work() {
    let run = run_script(
        "sleep 0.3 & p=$!\n( sleep 0.1 & q=$!; wait $q; echo \"own:$?\" )\nwait $p; echo \"outer:$?\"\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "own:0\nouter:0\n", "stdout: {}", run.stdout);
}
