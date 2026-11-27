//! rubash#2721 — FUNCNEST overflow does not abort the enclosing
//! command list.
//!
//! GNU ground truth (bash 5.3.20 oracle, probed from script files):
//! execute_cmd.c:5200-5206 execute_function — on overflow bash
//! prints the error, runs unwind-protects, resets funcnest = 0 and
//! jump_to_top_level (DISCARD): the rest of the current command
//! unit is discarded, the NEXT unit runs, and the discarded unit's
//! status is 1. rubash's execute_function (src/executor/
//! function_calls.rs) prints the same diagnostic and sets exit
//! code 1 but returns normally, so the list continues.
//!
//! The diagnostic text and the limit computation already agree in
//! both shells (issue evidence); the pins assert the unwind.

#![cfg(unix)]

use std::io::Write;
use std::process::{Command, Stdio};

fn run_script(body: &str) -> (String, String, Option<i32>) {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rubash-issue2721-{}-{}", std::process::id(), seq));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let script = dir.join("case.sh");
    let mut f = std::fs::File::create(&script).expect("write script");
    f.write_all(body.as_bytes()).expect("write body");
    drop(f);
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script)
        .stdin(Stdio::null())
        .output()
        .expect("spawn rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

const FUNCS: &str = "\
f(){ echo in-f; g; }\n\
g(){ echo in-g; h; }\n\
h(){ echo in-h; }\n";

/// The issue's reproducer: the `echo rc:$?` sharing the unit with
/// the overflowing call never runs; the next unit runs with the
/// limit lifted and succeeds.
#[test]
fn funcnest_overflow_discards_rest_of_unit() {
    let body = format!(
        "FUNCNEST=2\n{FUNCS}f; echo \"rc:$?\"\nFUNCNEST=0; f; echo \"rc2:$?\"\necho \"final:$?\"\n"
    );
    let (stdout, stderr, code) = run_script(&body);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(
        stdout, "in-f\nin-g\nin-f\nin-g\nin-h\nrc2:0\nfinal:0\n",
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("h: maximum function nesting level exceeded (2)"),
        "diagnostic must name the refused callee: {stderr}"
    );
}

/// Chain: the nesting counter is reset by the unwind (a later unit
/// overflows again at the same limit), the discarded unit settles
/// at status 1, and a DISCARD raised inside `eval` discards the
/// rest of the eval string but not the rest of the script.
#[test]
fn funcnest_discard_resets_counter_and_crosses_eval() {
    let body = format!(
        "FUNCNEST=2\n{FUNCS}f\necho \"unit2\"\nf\necho \"unit3 rc:$?\"\n\
         eval 'f; echo EVAL-AFTER'\necho \"after-eval rc:$?\"\n"
    );
    let (stdout, stderr, code) = run_script(&body);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(
        stdout, "in-f\nin-g\nunit2\nin-f\nin-g\nunit3 rc:1\nin-f\nin-g\nafter-eval rc:1\n",
        "stderr: {stderr}"
    );
    assert_eq!(
        stderr
            .matches("maximum function nesting level exceeded")
            .count(),
        3,
        "each unit overflows afresh (counter reset): {stderr}"
    );
    assert!(
        !stdout.contains("EVAL-AFTER"),
        "the rest of the eval string is discarded: {stdout}"
    );
}
