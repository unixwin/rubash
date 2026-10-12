//! issue#531 regression: a failing command inside a `{ }`, `for` or `case`
//! body must fire the ERR trap exactly once. GNU adjudicates the ERR trap
//! once per failing command (execute_cmd.c:851 computes the was_error_trap
//! state once per command in execute_command_internal, and compound bodies
//! hand their status up without a second adjudication), so the issue's
//! repro of three failing constructs prints E three times, not five.
//!
//! The double fire was fixed on the way by the errexit body adjudication
//! (the same post-execute_command gate now orders both the ERR fire and the
//! errexit promotion for if/while/until/for/case/select bodies); these
//! tests lock the observable contract so it cannot regress. On the audited
//! baseline 764b9dde the repro prints E five times ({ } twice, for once,
//! case twice).
//!
//! Oracle: GNU bash 5.3.0 (WSL) 2026-10-12 — every expectation below was
//! captured from it, including the no-fire contexts (if/while/until
//! conditions, negated pipelines, a redirected brace group, the left arm of
//! an AND-OR list), the no-inheritance rule for functions without errtrace,
//! the double fire a failing function produces WITH errtrace (body plus the
//! call itself), and the single fire on the set -e exit path.

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn shell_bin() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_rubash") {
        return PathBuf::from(path);
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(profile)
        .join(if cfg!(windows) {
            "rubash.exe"
        } else {
            "rubash"
        })
}

fn run_c(script: &str) -> (i32, Vec<String>) {
    let output = Command::new(shell_bin())
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|l| l.to_string())
        .collect();
    (output.status.code().unwrap_or(-1), stdout)
}

fn e_lines(lines: &[String]) -> usize {
    lines.iter().filter(|l| l.as_str() == "E").count()
}

/// The issue's exact repro: three constructs, three E lines.
#[test]
fn issue_repro_fires_three_not_five() {
    let (status, lines) = run_c(
        "trap 'echo E' ERR; { false; }; for i in 1; do false; done; case x in x) false;; esac; echo done",
    );
    assert_eq!(status, 0);
    assert_eq!(
        lines,
        vec!["E", "E", "E", "done"],
        "each construct fires once"
    );
}

/// The construct matrix, GNU-verified line for line: one fire per failing
/// body command; no fire for if/while/until conditions, a redirected brace
/// group, a negated pipeline, or a mid-body success; one fire for the
/// failed command-substitution assignment.
#[test]
fn construct_matrix_matches_gnu() {
    let script = r"trap 'echo E' ERR
{ false; }
echo end1
for i in 1; do false; done
echo end2
case x in x) false;; esac
echo end3
if false; then :; fi
echo end4
while false; do :; done
echo end5
until true; do :; done
echo end6
{ false; } >/dev/null
echo end7
! { false; }
echo end8
x=$(false)
echo end9
echo DONE";
    let (status, lines) = run_c(script);
    assert_eq!(status, 0);
    assert_eq!(
        lines,
        vec![
            "E", "end1", "E", "end2", "E", "end3", "end4", "end5", "end6", "end7", "end8", "E",
            "end9", "DONE"
        ]
    );
}

/// Repeated failures each fire once: two brace groups produce two E lines.
#[test]
fn repeated_failures_fire_once_each() {
    let (_, lines) = run_c("trap 'echo E' ERR; { false; }; { false; }; echo end");
    assert_eq!(lines, vec!["E", "E", "end"]);
}

/// Without errtrace the function BODY's failures fire nothing extra, but
/// the failed function call itself is a failing simple command at top level
/// and fires exactly once (GNU verified). The nested compounds inside the
/// body add no further fires.
#[test]
fn failing_function_fires_once_via_call_without_errtrace() {
    let (_, lines) = run_c(
        "trap 'echo E' ERR; f(){ { false; }; for i in 1; do false; done; case x in x) false;; esac; }; f; echo end",
    );
    assert_eq!(
        lines,
        vec!["E", "end"],
        "one fire for the call, none for the body"
    );
}

/// With errtrace a failing function fires twice — once for the failing body
/// command, once for the failed call itself (GNU bash behavior, verified).
#[test]
fn errtrace_failing_function_fires_body_and_call() {
    let (_, lines) = run_c("set -E; trap 'echo E' ERR; f(){ false; }; f; echo end");
    assert_eq!(lines, vec!["E", "E", "end"]);
}

/// Under `set -e` the ERR fire and the exit promotion each happen once: one
/// E, then the shell exits with the failed status.
#[test]
fn set_e_exit_fires_err_once() {
    let (status, lines) = run_c("set -e; trap 'echo E' ERR; { false; }; echo not-reached");
    assert_eq!(lines, vec!["E"], "one E, then errexit stops the script");
    assert_eq!(status, 1);
    assert!(!lines.iter().any(|l| l == "not-reached"));
}

/// AND-OR arms: the left arm of `||`/`&&` is exempt from the ERR trap (the
/// whole list fails only when the final arm fails).
#[test]
fn andor_left_arm_is_exempt() {
    let (_, lines) = run_c("trap 'echo E' ERR; { false; } || true; false || true; echo end");
    assert_eq!(lines, vec!["end"]);
    let (_, lines) = run_c("trap 'echo E' ERR; { false; } || false; echo end");
    assert_eq!(lines, vec!["E", "end"], "final failing arm fires once");
}

/// A failing command inside nested compound contexts (for-in-case) fires
/// once per failure.
#[test]
fn nested_compound_contexts_fire_once_each() {
    let (_, lines) = run_c(
        "trap 'echo E' ERR; for i in 1; do { false; }; done; case x in x) for j in 1; do false; done;; esac; echo end",
    );
    assert_eq!(lines, vec!["E", "E", "end"]);
}
