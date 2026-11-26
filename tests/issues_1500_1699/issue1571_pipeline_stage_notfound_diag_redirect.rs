//! rubash#1571: a pipeline stage in a NON-FIRST position whose
//! command is not found and which carries its own `2>file` loses the
//! "command not found" diagnostic entirely — GNU delivers it into the
//! stage's redirect target (the element's redirections run inside the
//! element). The first stage, a standalone command, and the `2>>`
//! forms already behaved; middle/last positions with `2>` did not.
//!
//! Oracle: GNU bash 5.3.20, script files, target pre-filled with a
//! SENTINEL line so a stray truncate is visible too.

#![cfg(unix)]

use std::process::{Command, Stdio};

/// Local stand-in for the `tempfile` crate (not a dev-dependency of
/// this repo): a unique scratch directory removed on drop.
struct ScratchDir(std::path::PathBuf);

impl ScratchDir {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch_tempdir() -> ScratchDir {
    let dir = std::env::temp_dir().join(format!(
        "rubash-scratch-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    ScratchDir(dir)
}

fn rubash_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_rubash"))
}

struct Case {
    dir: ScratchDir,
    stdout: String,
    stderr: String,
    status: i32,
}

fn run(script: &str) -> Case {
    let dir = scratch_tempdir();
    std::fs::write(dir.path().join("case.sh"), script).expect("write script");
    std::fs::write(dir.path().join("f"), "SENTINEL\n").expect("seed f");
    let output = Command::new(rubash_bin())
        .arg("case.sh")
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", dir.path())
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    Case {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        status: output.status.code().unwrap_or(-1),
        dir,
    }
}

fn f_contents(case: &Case) -> String {
    std::fs::read_to_string(case.dir.path().join("f")).expect("read f")
}

const DIAG_B: &str = "case.sh: line 1: nosuchB: command not found\n";
const DIAG_A: &str = "case.sh: line 1: nosuchA: command not found\n";

#[test]
fn issue1571_last_stage_notfound_diag_lands_in_its_redirect() {
    let case = run("nosuchA | nosuchB 2>f\n");
    assert_eq!(f_contents(&case), DIAG_B, "stderr: {:?}", case.stderr);
    assert_eq!(case.stderr, DIAG_A);
    assert_eq!(case.status, 127);
}

#[test]
fn issue1571_middle_stage_notfound_diag_lands_in_its_redirect() {
    let case = run("nosuchA | nosuchB 2>f | cat\n");
    assert_eq!(f_contents(&case), DIAG_B, "stderr: {:?}", case.stderr);
    assert_eq!(case.stderr, DIAG_A);
    assert_eq!(case.status, 0);
}

#[test]
fn issue1571_last_stage_after_successful_producer() {
    let case = run("/bin/true | nosuchB 2>f\n");
    assert_eq!(f_contents(&case), DIAG_B, "stderr: {:?}", case.stderr);
    assert_eq!(case.stderr, "");
    assert_eq!(case.status, 127);
}

#[test]
fn issue1571_first_stage_notfound_diag_control() {
    let case = run("nosuchB 2>f | cat\n");
    assert_eq!(f_contents(&case), DIAG_B, "stderr: {:?}", case.stderr);
    assert_eq!(case.stderr, "");
    assert_eq!(case.status, 0);
}

#[test]
fn issue1571_standalone_notfound_diag_control() {
    let case = run("nosuchB 2>f\n");
    assert_eq!(f_contents(&case), DIAG_B, "stderr: {:?}", case.stderr);
    assert_eq!(case.stderr, "");
    assert_eq!(case.status, 127);
}

#[test]
fn issue1571_last_stage_append_form_control() {
    let case = run("nosuchA | nosuchB 2>>f\n");
    assert_eq!(
        f_contents(&case),
        format!("SENTINEL\n{DIAG_B}"),
        "stderr: {:?}",
        case.stderr
    );
    assert_eq!(case.stderr, DIAG_A);
    assert_eq!(case.status, 127);
}

#[test]
fn issue1571_last_stage_with_stdout_redirect_too() {
    let case = run("nosuchA | nosuchB >o 2>f\n");
    assert_eq!(f_contents(&case), DIAG_B, "stderr: {:?}", case.stderr);
    assert_eq!(case.stderr, DIAG_A);
    let o = std::fs::read_to_string(case.dir.path().join("o")).expect("read o");
    assert_eq!(o, "");
    assert_eq!(case.status, 127);
    let _ = case.stdout;
}
