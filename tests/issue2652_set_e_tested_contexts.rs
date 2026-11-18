//! rubash#2652 — `set -e` failure suppression in tested contexts
//! (`&&`/`||`, `!`, conditions, `eval`) not honoured: the shell
//! exited at the first tested failure.
//!
//! GNU (bash 5.3, `third_party/bash/`): the -e check fires only at
//! a simple command's completion (execute_cmd.c:1004-1018,
//! `execute_command_internal`), at a pipeline's completion
//! (execute_cmd.c:2913, the `|` case of `execute_connection`), and
//! at a subshell's completion (execute_cmd.c:1170) — never at a
//! compound command's own completion, and never at the end of an
//! `&&`/`||` connection. The tested operands are marked
//! CMD_IGNORE_RETURN (command.h:182): `! CMD` gains it
//! (execute_cmd.c:652-656); the first `&&`/`||` operand is marked
//! (execute_cmd.c:2943, `execute_connection`); the second operand
//! only if the connection itself is ignore_return; the mark
//! propagates into compounds (execute_cmd.c:1024-1064 for
//! for/case/while/until/if, 2703/2829/3803). A compound's status
//! inherited from an exempt arm therefore never triggers -e.
//!
//! Rubash's whole-file fast path (`run_source`) was already correct
//! — it relies purely on the inner executor's per-command errexit
//! raises. The bug was the grouped driver
//! (`run_script_grouped_at_line_offset`): after each group it
//! re-adjudicated errexit from the group's terminal status
//! (`errexit_break_verdict`, script_driver.rs) — context-blind, so
//! a group like `false && echo A` ended status 1 from a suppressed
//! non-last operand and the driver stopped the run. The fix records
//! whether the unit's terminal status was produced while errexit
//! was suppressed (`errexit_suppressed_terminal`, set by
//! `with_errexit_suppressed`, cleared at `execute_command` entry
//! for an unsuppressed dispatch so a later own-status command
//! cannot inherit an earlier operand's exemption), exempts the
//! verdict on that record, and counts an armed errexit unwind as
//! a stop (the inner gates already made the context-aware
//! decision; `false || false` must die even though the verdict
//! reads the first `false`'s exemption).
//!
//! Oracle: the C source above is the specification. The expected
//! outputs below were also checked for consistency against the
//! local /usr/bin/bash 5.2.21 (script-file and stdin probes) —
//! that is a consistency check only, not the basis for the rule,
//! and a WSL GNU Bash 5.3 binary probe on this Linux VM is OWED
//! (no WSL here; building a 5.3 binary from third_party/bash is
//! the remaining verification step):
//!   set -e; false && echo A-ran; echo AND-survived;
//!   false || echo OR-survived; ! false; echo NOT-survived;
//!   if eval false; then echo oops; fi; echo EVAL-survived
//!     -> all four lines printed, rc 0.
//!   false || false; echo B -> rc 1, no B.
//!   false && echo A; false; echo B -> rc 1, no B.
//!   true && if true; then false && true; fi; echo B -> rc 0, B.
//!   true && { false && echo A; }; echo B -> rc 0, B.
//!   set -eE; trap 'echo TRAPPED' ERR; false && echo A; echo B
//!     -> rc 0, no TRAPPED, B.
//!
//! These pins exercise the GROUPED path (script files containing
//! `eval`, which main.rs routes to the grouped driver) and stdin —
//! never `-c` or a bare fast-path file, which were already correct
//! and would pass on the unmodified tree. The `eval true` line at
//! the top of the shaped files is the routing trigger, not part of
//! the assertion.
//!
//! Fail-first (2026-11-11, unmodified HEAD f5f65d3, src changes
//! stashed, pins in place): 5 passed, 5 failed. RED as expected —
//! issue_repro_grouped, issue_repro_stdin,
//! and_or_first_operand_survives_grouped,
//! last_operand_compound_inherits_exemption_grouped,
//! err_trap_not_fired_for_suppressed_grouped (all exit 1, no
//! output). GREEN on the unmodified tree, as expected — the four
//! die-cases (plain_and_last_operand, or_last_operand_grouped,
//! or_last_operand_stdin, and_then_plain) already died correctly
//! pre-fix, and not_and_conditions_survives_grouped (its groups
//! end status 0: `! false` and the conditions exit 0, so the
//! context-blind verdict never fired). After the fix all 10 pass.

#![cfg(unix)]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct ScratchDir(PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(case: &str) -> ScratchDir {
    let dir = std::env::temp_dir().join(format!("rubash-2652-{case}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch");
    ScratchDir(dir)
}

/// Spawn rubash on `script` as a FILE inside `dir`; the `eval true`
/// line routes the file through the grouped driver. Returns
/// (stdout, stderr, status code).
fn run_rubash_file(dir: &Path, script: &str) -> (String, String, Option<i32>) {
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write probe script");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg(&path)
        .current_dir(dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    run_child(cmd)
}

/// Spawn rubash reading `script` from stdin. Returns
/// (stdout, stderr, status code).
fn run_rubash_stdin(dir: &Path, script: &str) -> (String, String, Option<i32>) {
    use std::io::Write;
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.current_dir(dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(script.as_bytes())
        .expect("write stdin");
    finish(child)
}

fn run_child(mut cmd: Command) -> (String, String, Option<i32>) {
    let child = cmd.spawn().expect("spawn rubash");
    finish(child)
}

fn finish(mut child: std::process::Child) -> (String, String, Option<i32>) {
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        match child.try_wait().expect("poll rubash") {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                panic!("rubash did not exit within 30s");
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    (stdout, stderr, status.code())
}

/// The issue's own repro, verbatim (its `if eval false` line is
/// the grouped routing trigger).
fn issue_repro() -> &'static str {
    "set -e\nfalse && echo A-ran\necho AND-survived\nfalse || echo OR-survived\n! false\necho NOT-survived\nif eval false; then echo oops; fi\necho EVAL-survived\n"
}

const REPRO_OUT: &str = "AND-survived\nOR-survived\nNOT-survived\nEVAL-survived\n";

// ---------------------------------------------------------------- repro

#[test]
fn issue2652_issue_repro_grouped() {
    let dir = scratch("repro-file");
    let (stdout, stderr, code) = run_rubash_file(&dir.0, issue_repro());
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, REPRO_OUT, "stderr:\n{stderr}");
}

#[test]
fn issue2652_issue_repro_stdin() {
    let dir = scratch("repro-stdin");
    let (stdout, stderr, code) = run_rubash_stdin(&dir.0, issue_repro());
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, REPRO_OUT, "stderr:\n{stderr}");
}

// ---------------------------------------------------------------- survive

#[test]
fn issue2652_and_or_first_operand_survives_grouped() {
    let dir = scratch("and-first");
    let script = "set -e\neval true\nfalse && echo A\necho B\nfalse || echo C\necho D\n";
    let (stdout, stderr, code) = run_rubash_file(&dir.0, script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "B\nC\nD\n", "stderr:\n{stderr}");
}

#[test]
fn issue2652_not_and_conditions_survive_grouped() {
    let dir = scratch("not-cond");
    let script = "set -e\neval true\n! false\necho B\nif false; then echo oops; fi\necho C\nwhile false; do echo oops; done\necho D\nuntil true; do echo oops; done\necho E\n";
    let (stdout, stderr, code) = run_rubash_file(&dir.0, script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "B\nC\nD\nE\n", "stderr:\n{stderr}");
}

#[test]
fn issue2652_last_operand_compound_inherits_exemption_grouped() {
    // GNU survives both: the compound's status is inherited from
    // an exempt arm, and a compound's own completion never
    // triggers -e (execute_cmd.c has no post-check for compounds).
    let dir = scratch("compound-last");
    let script = "set -e\neval true\ntrue && if true; then false && true; fi\necho B\ntrue && { false && echo A; }\necho C\n";
    let (stdout, stderr, code) = run_rubash_file(&dir.0, script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "B\nC\n", "stderr:\n{stderr}");
}

#[test]
fn issue2652_err_trap_not_fired_for_suppressed_grouped() {
    let dir = scratch("err-trap");
    let script = "set -eE\neval true\ntrap 'echo TRAPPED' ERR\nfalse && echo A\necho B\n";
    let (stdout, stderr, code) = run_rubash_file(&dir.0, script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "B\n", "stderr:\n{stderr}");
}

// ---------------------------------------------------------------- die

#[test]
fn issue2652_plain_and_last_operand_fail_dies_grouped() {
    let dir = scratch("plain-die");
    let script = "set -e\neval true\ntrue && false\necho B\n";
    let (stdout, stderr, code) = run_rubash_file(&dir.0, script);
    assert_eq!(code, Some(1), "stderr:\n{stderr}");
    assert_eq!(stdout, "", "stderr:\n{stderr}");
    let dir = scratch("plain-die-2");
    let script = "set -e\neval true\nfalse\necho B\n";
    let (stdout, stderr, code) = run_rubash_file(&dir.0, script);
    assert_eq!(code, Some(1), "stderr:\n{stderr}");
    assert_eq!(stdout, "", "stderr:\n{stderr}");
}

#[test]
fn issue2652_or_last_operand_fail_dies_grouped() {
    // The regression the first fix attempt introduced: the verdict
    // read the first `false`'s exemption while the last `false`
    // had already raised inside. The armed unwind is a stop.
    let dir = scratch("or-last");
    let script = "set -e\neval true\nfalse || false\necho B\n";
    let (stdout, stderr, code) = run_rubash_file(&dir.0, script);
    assert_eq!(code, Some(1), "stderr:\n{stderr}");
    assert_eq!(stdout, "", "stderr:\n{stderr}");
}

#[test]
fn issue2652_and_then_plain_fail_dies_grouped() {
    let dir = scratch("and-then-plain");
    let script = "set -e\neval true\nfalse && echo A; false\necho B\n";
    let (stdout, stderr, code) = run_rubash_file(&dir.0, script);
    assert_eq!(code, Some(1), "stderr:\n{stderr}");
    assert_eq!(stdout, "", "stderr:\n{stderr}");
}

#[test]
fn issue2652_or_last_operand_fail_dies_stdin() {
    let dir = scratch("or-last-stdin");
    let (stdout, stderr, code) = run_rubash_stdin(&dir.0, "set -e\nfalse || false\necho B\n");
    assert_eq!(code, Some(1), "stderr:\n{stderr}");
    assert_eq!(stdout, "", "stderr:\n{stderr}");
}
