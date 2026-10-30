//! Issue unixwin/rubash#1184 f4: a process-substitution body that fails
//! inside a SPAWNED same-exe child reported its diagnostics at line 1 —
//! the child only received the script name, never the line that
//! constructed the substitution.
//!
//! GNU owner: subst.c:6362 process_substitute() forks the body; the
//! fork inherits the parent's line_number at substitution time, so a
//! redirect failing inside the body (`3: Bad file descriptor` for
//! `>(tr a-z A-Z >&3)` beside a command whose own fd 3 is bound only
//! for the outer command) reports the CONSTRUCTION line. Rubash's
//! spawn carriers now seed that line through the `&` respawn's env
//! protocol (__RUBASH_CURRENT_LINE + __RUBASH_LINE_OFFSET,
//! compound_exec.rs): the output spawn captures the owning command's
//! line at materialization (external_setup.rs), the conditional-operand
//! queue captures the `[[` line at realization (conditional.rs), and
//! the `exec`-redirect input bind passes its command's line
//! (trap_exec.rs).
//!
//! Also pinned, as already-correct controls: the in-process input path
//! (f4b) and the compound-command persistent path (f4comp), which seed
//! from the executing command's CURRENT_LINE (rubash#1024) and must not
//! move.
//!
//! Verified against GNU Bash 5.3.20 script-file probes (f4/f4late/
//! f4exec/f4cond report lines 4/6/4/4; f4b lines 2 + 4; f4comp line 4).
//! Note the platform split recorded in the R-cluster report: GNU's
//! `rc=141` for f4 comes from the external `tee` dying on the dead
//! /dev/fd operand, a materialization difference — only the body's
//! diagnostic attribution is asserted here.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_rubash_file(script: &str, tag: &str) -> (String, String, Option<i32>, String) {
    let path = std::env::temp_dir().join(format!("issue1184_f4_{}_{}.sh", tag, std::process::id()));
    std::fs::write(&path, script).expect("write probe script");
    let path_text = path.to_string_lossy().into_owned();
    let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let out = child.wait_with_output().expect("wait rubash");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
        path_text,
    )
}

fn run_stdin(script: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(script.as_bytes())
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

const F4: &str =
    "echo line2\necho line3\necho line4\ntee >(tr a-z A-Z >&3) <<< \"lower\" 3>&1\nwait\n";

#[test]
fn output_procsub_body_diag_reports_construction_line() {
    let (_stdout, stderr, _code, path) = run_rubash_file(F4, "f4");
    assert!(
        stderr.contains(&format!("{path}: line 4: 3: Bad file descriptor")),
        "stderr was: {stderr}"
    );
    assert!(
        !stderr.contains("line 1: 3: Bad file descriptor"),
        "stderr was: {stderr}"
    );
}

#[test]
fn output_procsub_body_diag_reports_late_construction_line() {
    let script =
        "echo a\necho b\necho c\necho d\necho e\ntee >(tr a-z A-Z >&3) <<< \"lower\" 3>&1\nwait\n";
    let (_stdout, stderr, _code, path) = run_rubash_file(script, "f4late");
    assert!(
        stderr.contains(&format!("{path}: line 6: 3: Bad file descriptor")),
        "stderr was: {stderr}"
    );
}

#[test]
fn output_procsub_body_diag_reports_line_via_stdin() {
    // Same attribution when the parent shell itself reads a pipe: the
    // construction line is the tee line (line 1 here).
    let script = "tee >(tr a-z A-Z >&3) <<< \"lower\" 3>&1\nwait\n";
    let (_stdout, stderr, _code) = run_stdin(script);
    assert!(
        stderr.contains("line 1: 3: Bad file descriptor"),
        "stderr was: {stderr}"
    );
}

#[test]
fn exec_input_procsub_body_diag_reports_construction_line_and_name() {
    let script = "echo a\necho b\necho c\nexec 3< <(echo hi >&3)\ncat <&3\necho rc=$?\n";
    let (_stdout, stderr, _code, path) = run_rubash_file(script, "f4exec");
    assert!(
        stderr.contains(&format!("{path}: line 4: 3: Bad file descriptor")),
        "stderr was: {stderr}"
    );
}

#[test]
fn conditional_output_procsub_body_diag_reports_construction_line() {
    let script =
        "echo a\necho b\necho c\n[[ -w >(tr a-z A-Z >&3) ]]\nsleep 0.3\nwait\necho rc=$?\n";
    let (_stdout, stderr, _code, path) = run_rubash_file(script, "f4cond");
    assert!(
        stderr.contains(&format!("{path}: line 4: 3: Bad file descriptor")),
        "stderr was: {stderr}"
    );
}

#[test]
fn input_procsub_body_diag_lines_unchanged() {
    // In-process input bodies already attributed correctly (control).
    let script =
        "echo line2\ncat <(echo hi >&3)\necho \"rc=$?\"\ncat <(nosuchcmd_xyz)\necho \"rc2=$?\"\n";
    let (_stdout, stderr, _code, path) = run_rubash_file(script, "f4b");
    assert!(
        stderr.contains(&format!("{path}: line 2: 3: Bad file descriptor")),
        "stderr was: {stderr}"
    );
    assert!(
        stderr.contains(&format!("{path}: line 4: nosuchcmd_xyz: command not found")),
        "stderr was: {stderr}"
    );
}

#[test]
fn compound_output_procsub_body_diag_line_unchanged() {
    // Compound-command output bodies run in-process at finish with the
    // enclosing command's line (control).
    let script =
        "echo a\necho b\necho c\nif :; then :; fi > >(tr a-z A-Z >&3)\nsleep 0.3\necho rc=$?\n";
    let (_stdout, stderr, _code, path) = run_rubash_file(script, "f4comp");
    assert!(
        stderr.contains(&format!("{path}: line 4: 3: Bad file descriptor")),
        "stderr was: {stderr}"
    );
}
