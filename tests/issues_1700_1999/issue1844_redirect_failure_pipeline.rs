//! rubash#1844: a pipeline element whose output redirection failed to
//! open used to unwind the WHOLE pipeline as an executor error — later
//! elements never ran, earlier elements' output was dropped, the
//! pipeline status collapsed to 1 and PIPESTATUS was never written.
//! GNU redir.c:149-176 (redirection_error) + redir.c:246
//! (do_redirections): the failed open reports through the fd-2 binding
//! in force and fails only THAT element; the pipeline status is the
//! last element's and PIPESTATUS carries every element.
//!
//! Expectations byte-verified against GNU bash 5.3.20 (script-file
//! probes, 2026-10-27).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

const BAD: &str = "/nonexist_1844/e";

#[test]
fn issue1844_first_stage_failure_later_stages_still_run() {
    let (stdout, stderr, code) = rubash(&format!(
        "echo hi 2>{BAD} | cat | wc -c; echo \"st=$? ps=[${{PIPESTATUS[*]}}]\""
    ));
    assert_eq!(stdout, "0\nst=0 ps=[1 0 0]\n");
    assert_eq!(
        stderr.matches("No such file or directory").count(),
        1,
        "diagnostic must print exactly once: {stderr:?}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue1844_middle_stage_failure_keeps_neighbours() {
    let (stdout, _, _) = rubash(&format!(
        "echo hi | echo yo 2>{BAD} | cat; echo \"st=$? ps=[${{PIPESTATUS[*]}}]\""
    ));
    assert_eq!(stdout, "st=0 ps=[0 1 0]\n");
}

#[test]
fn issue1844_last_stage_failure_is_the_pipeline_status() {
    let (stdout, _, _) = rubash(&format!(
        "echo hi | nosuchcmd_xyz_1844 2>{BAD}; echo \"st=$? ps=[${{PIPESTATUS[*]}}]\""
    ));
    assert_eq!(stdout, "st=1 ps=[0 1]\n");
}

#[test]
fn issue1844_command_not_found_with_failing_redirect_reports_once() {
    // First stage, unresolvable name AND failing stderr redirect: the
    // redirect diagnostic belongs to the element and prints once (the
    // speculative concurrent-path probe must not report it too).
    let (stdout, stderr, _) = rubash(&format!(
        "nosuchcmd_xyz_1844 2>{BAD} | cat; echo \"st=$? ps=[${{PIPESTATUS[*]}}]\""
    ));
    assert_eq!(stdout, "st=0 ps=[1 0]\n");
    assert_eq!(
        stderr.matches("No such file or directory").count(),
        1,
        "diagnostic must print exactly once: {stderr:?}"
    );
}

#[test]
fn issue1844_input_redirect_failure_unaffected() {
    let (stdout, _, _) =
        rubash("cat < /nonexist_1844/e | wc -c; echo \"st=$? ps=[${PIPESTATUS[*]}]\"");
    assert_eq!(stdout, "0\nst=0 ps=[1 0]\n");
}

#[test]
fn issue1844_top_level_redirect_failure_status_and_stop_walk() {
    // Solo builtin: status 1 (GNU), and the ordered walk stops at the
    // failing entry — a later `>ok` entry must NOT create its file,
    // while an earlier one DOES (created empty).
    let dir = std::env::temp_dir().join(format!("rubash1844-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir scratch");
    let ok1 = dir.join("ok1.txt");
    let ok2 = dir.join("ok2.txt");
    let script = format!(
        "echo hi >{} 2>{BAD}; echo \"a=$?\"; echo hi 2>{BAD} >{}; echo \"b=$?\"",
        ok1.display(),
        ok2.display()
    );
    let (stdout, _, _) = rubash(&script);
    assert_eq!(stdout, "a=1\nb=1\n");
    assert!(ok1.exists(), "entry before the failure must be applied");
    assert_eq!(std::fs::read_to_string(&ok1).unwrap(), "");
    assert!(!ok2.exists(), "walk must stop at the failing entry");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue1844_top_level_buffered_builtin_failure_status() {
    // A buffered builtin whose body would succeed still fails with 1
    // when its output redirection cannot open (the command never ran).
    let (stdout, stderr, _) = rubash(&format!("ulimit -a 2>{BAD}; echo \"st=$?\""));
    assert_eq!(stdout, "st=1\n");
    assert_eq!(
        stderr.matches("No such file or directory").count(),
        1,
        "diagnostic must print exactly once: {stderr:?}"
    );
    let (stdout, _, _) = rubash(&format!("printf 'X\\n' 2>{BAD}; echo \"st=$?\""));
    assert_eq!(
        stdout, "st=1\n",
        "printf output must be discarded, status 1"
    );
}
