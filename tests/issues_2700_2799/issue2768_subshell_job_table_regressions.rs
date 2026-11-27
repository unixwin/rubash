//! rubash#2768 (narrowed residue): the paren subshell's `jobs` READ
//! face.
//!
//! GNU ground truth (bash 5.3.20): a `( )` subshell starts with an
//! EMPTY job table — execute_in_subshell runs without_job_control
//! (jobs.c:5356-5366), which calls delete_all_jobs(0) (jobs.c:5363).
//! The disown face was fixed by rubash#2897 (`clear_for_subshell` at
//! the subshell entries); this pin guards the read face with the
//! issue's exact regression seed, verbatim.
//!
//! NOTE: this stem is green on master 500769ef already — #2897's
//! clear fixed both faces after the issue was filed against
//! 568dce36. It is kept as the regression nail for the read face.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_script_file(body: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-2768-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("seed.sh");
    {
        let mut file = std::fs::File::create(&path).expect("write script");
        file.write_all(body.as_bytes()).expect("write script");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let rc = output.status.code();
    let _ = std::fs::remove_dir_all(&dir);
    (stdout, stderr, rc)
}

/// The issue's regression seed, verbatim: the subshell's `jobs`
/// count is 0 and its bare `disown` fails rc 1.
#[test]
fn subshell_jobs_read_face_starts_empty() {
    let (stdout, stderr, rc) = run_script_file(
        "sleep 5 & p=$!\n\
         (jobs | wc -l) > subcount.txt\n\
         read -r n < subcount.txt\n\
         if [ \"$n\" != 0 ]; then echo \"FAIL subshell jobs count: got $n, want 0\"; kill $p 2>/dev/null; exit 1; fi\n\
         (disown 2>disown.err; echo $? > disown.rc)\n\
         rc=$(cat disown.rc)\n\
         if [ \"$rc\" != 1 ]; then echo \"FAIL subshell disown rc: got $rc, want 1\"; kill $p 2>/dev/null; exit 1; fi\n\
         kill $p 2>/dev/null; wait $p 2>/dev/null\n\
         echo PASS\n",
    );
    assert_eq!(rc, Some(0), "seed stderr: {stderr:?}");
    assert!(
        stdout.contains("PASS"),
        "seed must PASS: stdout {stdout:?} stderr {stderr:?}"
    );
}

/// A subshell's own job is the only one it lists (parent 1 +
/// subshell self 1 -> 1; parent 2 + self 1 -> 1).
#[test]
fn subshell_lists_only_its_own_jobs() {
    for (parent_jobs, want) in [(1, "1"), (2, "1")] {
        let mut body = String::new();
        for _ in 0..parent_jobs {
            body.push_str("sleep 5 &\n");
        }
        body.push_str("(sleep 4 & jobs | wc -l)\nwait\n");
        let (stdout, _, rc) = run_script_file(&body);
        assert_eq!(rc, Some(0));
        assert!(
            stdout.trim() == want,
            "parent {parent_jobs} + subshell self 1: want {want}, got {stdout:?}"
        );
    }
}

/// Positive control: command substitution INHERITS the parent table
/// (GNU and rubash both report 1) — only `( )` starts empty.
#[test]
fn command_substitution_inherits_parent_job_table() {
    let (stdout, _, rc) = run_script_file("sleep 5 & x=$(jobs | wc -l); echo $x; wait\n");
    assert_eq!(rc, Some(0));
    assert!(
        stdout.trim() == "1",
        "comsub must see the parent job: {stdout:?}"
    );
}
