//! Job-entry lifetime after an operand `wait` consumes the entry
//! (rubash#2529; GNU bash 5.3.20 oracle-verified 2026-11-02).
//!
//! GNU's lifetime law: an operand wait only MARKS its job
//! J_NOTIFIED (jobs.c:3434-3437 wait_for_job, jobs.c:2776-2779
//! wait_for_single_pid); deletion is cleanup_dead_jobs
//! (jobs.c:1303, DEADJOB && IS_NOTIFIED), which runs at
//! stop_pipeline — every pipeline launch (jobs.c:572) — and at
//! notify_and_cleanup (jobs.c:3642), reached from read_command
//! (parse.y:2533), wait_for's tail (jobs.c:3385) and the listing
//! builtins. So after `wait` consumes an entry, the NEXT created
//! job reuses its number and a later jobspec operand on a new line
//! resolves "no such job", while the entry survives between the
//! `;` elements of one line and unwaited dead jobs are retained
//! (the rubash#2381/#2340 contract). Rubash never collected at
//! creation or at the reader boundary: the next job was numbered
//! [2], `kill %n` silently succeeded against the corpse, and a
//! following bare `wait` rode out the misnumbered live job.

#![cfg(unix)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn rubash_bin() -> String {
    env!("CARGO_BIN_EXE_rubash").to_string()
}

/// Run a script FILE under the built rubash; return (stdout, stderr).
fn run_script(tag: &str, body: &str) -> (String, String) {
    let dir = std::env::temp_dir().join(format!("issue2529-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    let path: PathBuf = dir.join("case.sh");
    fs::write(&path, body).expect("write script");
    let output = Command::new(rubash_bin())
        .arg(&path)
        .current_dir(&dir)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn issue2529_consumed_entry_frees_its_number_and_jobspec() {
    // Cross-line faces (adj-signals §issue1991/§issue2381): after
    // `wait %1`, the consumed slot answers "no such job" and the
    // next job takes %1 for real.
    let (stdout, stderr) = run_script(
        "reuse",
        "sleep 30 & kill %1\n\
         wait %1; echo w1=$?\n\
         kill -TERM %1; echo kold=$?\n\
         sleep 30 &\n\
         kill -TERM %1; echo knew=$?\n\
         wait %1; echo w2=$?\n",
    );
    assert_eq!(stdout, "w1=143\nkold=1\nknew=0\nw2=143\n");
    assert!(
        stderr.contains("kill: %1: no such job"),
        "stderr should report no such job, got: {stderr}"
    );
}

#[test]
fn issue2529_creation_collects_within_one_line() {
    // stop_pipeline (jobs.c:572) collects at creation even inside
    // one list: the second `sleep &` reuses [1] on the same line,
    // so `kill -TERM %1` reaches the live job.
    let (stdout, stderr) = run_script(
        "sameline",
        "sleep 30 & kill %1; wait %1; sleep 30 & kill -TERM %1; echo k=$?; wait %1; echo w=$?\n",
    );
    assert_eq!(stdout, "k=0\nw=143\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2529_repeated_wait_on_later_line_is_no_such_job() {
    // The reader boundary (parse.y:2533) collects the wait-marked
    // entry, so the same jobspec waited again on the NEXT line is
    // 127 "no such job" — while an unwaited sibling still answers
    // from its retained entry (and its status survives in the
    // completed-status table after the listing collects it).
    let (stdout, stderr) = run_script(
        "rewait",
        "sleep 30 & sleep 30 & kill %1 %2\n\
         wait %1; echo rc1=$?\n\
         wait %1; echo rc2=$?\n\
         wait %2; echo rc3=$?\n\
         jobs | wc -l\n",
    );
    assert_eq!(stdout, "rc1=143\nrc2=127\nrc3=143\n0\n");
    assert!(
        stderr.contains("wait: %1: no such job"),
        "stderr should report no such job, got: {stderr}"
    );
}

#[test]
fn issue2529_kill_after_pid_wait_and_bare_wait_promptness() {
    // adj-signals §issue1604 shape, shortened: `wait $p` consumes
    // the entry, so a second `kill -KILL %1` is "no such job", the
    // replacement job takes %1 (k3 reaches it), and the bare wait
    // returns at once instead of riding out the replacement.
    let (stdout, stderr) = run_script(
        "barewait",
        "sleep 10 & p=$!\n\
         kill -KILL %1\n\
         wait $p 2>/dev/null; echo w=$?\n\
         kill -KILL %1; echo k2=$?\n\
         sleep 10 &\n\
         kill -KILL %1; echo k3=$?\n\
         wait 2>/dev/null; echo bare=$?\n",
    );
    assert_eq!(stdout, "w=137\nk2=1\nk3=0\nbare=0\n");
    assert!(
        stderr.contains("kill: %1: no such job"),
        "stderr should report no such job, got: {stderr}"
    );
}
