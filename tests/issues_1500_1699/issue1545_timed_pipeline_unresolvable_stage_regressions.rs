//! rubash#1545 — a timed pipeline's later stage "failure" was dropped:
//! `PATH=/nonexistent; time -p true | cat` reported rc 0 and no
//! diagnostic where GNU bash 5.3.20 reports `cat: command not found`,
//! still prints the timing, and the pipeline exits 127.
//!
//! Root cause: the stage was never dropped. It EXECUTED — in-process.
//! rubash emulates a set of external commands (pipeline inline stages
//! head/cat/sed/grep/wc/tr, the external file builtins cat/sed/mkdir/
//! cp/rm/..., the identity tools, the sleep/dirname/basename fast
//! paths, the comsub shortcuts), and every emulation admission shared
//! one gate: stand down only when PATH is UNSET (rubash#1126). Under a
//! SET PATH that simply does not hold the command, GNU's
//! findcmd.c:348-427 search_for_command fails the lookup, while the
//! emulations kept executing a binary that does not exist — so the
//! command's existence depended on the invocation shape.
//!
//! The fix widens the shared gate to GNU's rule
//! (`Executor::emulated_external_stands_down`): an emulation stands in
//! for a PATH binary and must stand down whenever that binary does not
//! resolve — hash-table hit or PATH walk hit — so the real dispatch
//! reports the failure. Expectations below are the GNU oracle's.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn rubash_c(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("rubash-1545-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch dir");
        Scratch(dir)
    }

    fn rubash_file(&self, script: &str) -> (String, String, Option<i32>) {
        let file = self.0.join("case.sh");
        fs::write(&file, script).expect("write script");
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&file)
            .current_dir(&self.0)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.0)
            .output()
            .expect("run rubash");
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
        )
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// ---------------------------------------------------------------------
// The filed form and its timing family.
// ---------------------------------------------------------------------

#[test]
fn issue1545_timed_pipeline_last_stage_not_found() {
    let (stdout, stderr, code) = rubash_c("PATH=/nonexistent; time -p true | cat; echo rc=$?");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=127\n", "stderr: {stderr}");
    assert!(
        stderr.contains("cat: command not found"),
        "stderr: {stderr}"
    );
    // The timing report still prints — the pipeline ran, its last
    // stage's lookup failed.
    assert!(stderr.contains("real"), "expected a time report: {stderr}");
}

#[test]
fn issue1545_bare_time_pipeline_last_stage_not_found() {
    let (stdout, stderr, code) = rubash_c("PATH=/nonexistent; time true | cat; echo rc=$?");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=127\n", "stderr: {stderr}");
    assert!(
        stderr.contains("cat: command not found"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("real"), "expected a time report: {stderr}");
}

/// `!` inverts the pipeline status: GNU's 127 becomes 0.
#[test]
fn issue1545_negated_timed_pipeline_inverts_the_127() {
    let (stdout, stderr, code) = rubash_c("PATH=/nonexistent; ! time -p true | cat; echo rc=$?");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr: {stderr}");
    assert!(
        stderr.contains("cat: command not found"),
        "stderr: {stderr}"
    );
}

/// The timed command itself is the first stage and does not resolve;
/// the last stage still runs on the empty stream.
#[test]
fn issue1545_timed_first_stage_not_found_last_stage_still_runs() {
    let (stdout, stderr, code) =
        rubash_c("PATH=/nonexistent; time -p cat /etc/hostname | wc -c; echo rc=$?");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=127\n", "stderr: {stderr}");
    assert!(
        stderr.contains("cat: command not found"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("wc: command not found"), "stderr: {stderr}");
    assert!(stderr.contains("real"), "expected a time report: {stderr}");
}

#[test]
fn issue1545_timed_pipeline_in_group_and_function() {
    let scratch = Scratch::new("group-fn");
    let (stdout, stderr, code) = scratch.rubash_file(
        "PATH=/nonexistent\n{ time -p true | cat; echo group=$?; }\nf() { time -p true | cat; echo fn=$?; }\nf\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "group=127\nfn=127\n", "stderr: {stderr}");
    assert_eq!(
        stderr.matches("cat: command not found").count(),
        2,
        "stderr: {stderr}"
    );
}

// ---------------------------------------------------------------------
// Controls: resolvable stages keep working exactly as before.
// ---------------------------------------------------------------------

#[test]
fn issue1545_timed_pipeline_normal_last_stage() {
    let (stdout, stderr, code) = rubash_c("time -p echo hi | cat; echo rc=$?");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "hi\nrc=0\n", "stderr: {stderr}");
    assert!(stderr.contains("real"), "expected a time report: {stderr}");
}

#[test]
fn issue1545_timed_pipeline_first_stage_failure_is_a_control() {
    // First stage unresolvable, last stage fine: the diagnostic names
    // the first stage and the pipeline status is the last stage's (0).
    let (stdout, stderr, code) = rubash_c("time -p nosuchcmd1545 | cat; echo rc=$?");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr: {stderr}");
    assert!(
        stderr.contains("nosuchcmd1545: command not found"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("real"), "expected a time report: {stderr}");
}

/// GNU flushes the whole command hash table on ANY PATH assignment
/// (variables.c sv_path -> phash_flush), so a `hash -p` entry recorded
/// before `PATH=/nonexistent` does NOT rescue the stage: GNU reports
/// the same 127 as the filed form. The sv_path watcher landed with
/// rubash#1551 (flush at every PATH bind funnel), activating this pin.
#[test]
fn issue1545_hashed_name_dies_with_path_assignment() {
    let (stdout, stderr, code) =
        rubash_c("hash -p /usr/bin/cat cat; PATH=/nonexistent; time -p true | cat; echo rc=$?");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=127\n", "stderr: {stderr}");
    assert!(
        stderr.contains("cat: command not found"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("real"), "expected a time report: {stderr}");
}

/// PATH unset keeps the #1126/#1145 behaviour byte-for-byte: the
/// failure is the execve ENOENT text, not "command not found".
#[test]
fn issue1545_unset_path_keeps_enoent_wording() {
    let (stdout, stderr, code) = rubash_c("unset PATH; time -p true | cat; echo rc=$?");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=127\n", "stderr: {stderr}");
    assert!(
        stderr.contains("cat: No such file or directory"),
        "stderr: {stderr}"
    );
}

// ---------------------------------------------------------------------
// The same admission invariant on the sibling emulation surfaces.
// ---------------------------------------------------------------------

/// Untimed pipelines share the stage gate.
#[test]
fn issue1545_untimed_pipeline_stage_not_found() {
    let (stdout, stderr, code) = rubash_c("PATH=/nonexistent; echo hi | head -1; echo rc=$?");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=127\n", "stderr: {stderr}");
    assert!(
        stderr.contains("head: command not found"),
        "stderr: {stderr}"
    );
}

/// The timed single command form routes through the file-builtin gate.
#[test]
fn issue1545_timed_single_cat_not_found() {
    let (stdout, stderr, code) =
        rubash_c("PATH=/nonexistent; time -p cat /etc/hostname; echo rc=$?");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=127\n", "stderr: {stderr}");
    assert!(
        stderr.contains("cat: command not found"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("real"), "expected a time report: {stderr}");
}

#[test]
fn issue1545_standalone_emulated_names_not_found() {
    let (stdout, stderr, code) = rubash_c(
        "PATH=/nonexistent; cat /etc/hostname; echo cat=$?; sleep 0; echo sleep=$?; uname; echo uname=$?",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout, "cat=127\nsleep=127\nuname=127\n",
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("cat: command not found"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("sleep: command not found"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("uname: command not found"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1545_comsub_cat_not_found() {
    let (stdout, stderr, code) =
        rubash_c("PATH=/nonexistent; x=$(echo hi | cat); echo \"[$x] rc=$?\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "[] rc=127\n", "stderr: {stderr}");
    assert!(
        stderr.contains("cat: command not found"),
        "stderr: {stderr}"
    );
}
