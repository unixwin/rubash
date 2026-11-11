//! ERR trap and `set -e` on a compound command's own redirection
//! error (rubash#2659; GNU bash 5.3.20 oracle-verified 2026-11-09).
//!
//! GNU `execute_command_internal` (execute_cmd.c:854-883) applies a
//! compound command's redirections BEFORE dispatch; a failure there
//! is an ordinary command failure with status 1: the ERR trap runs
//! (was_error_trap gate) and the errexit consequence follows
//! (jump_to_top_level ERREXIT), exactly like the simple-command site
//! at execute_cmd.c:993-1010. Rubash let the compound wrappers'
//! redirect failure bubble up as an executor error (or only recorded
//! the `compound_redirect_failed` status-flip mark), bypassing the
//! dispatch-level ERR/errexit adjudication in
//! `maybe_run_error_trap` — so neither the trap nor `set -e` fired.
//!
//! Script-file pins; the diagnostic line carries the script path and
//! is normalized to `S`.
//!
//! The timed pins at the bottom cover the PR #3269 review R1
//! regression: under a `time` prefix, the funnel conversion and the
//! absorption-site adjudication recorded the failure with
//! `err_trap_fired_at_dispatch` — a flag whose meaning is "the ERR
//! trap already fired at dispatch" (the #2117 suppression gate) —
//! without firing, so the ast_exec time branch's own adjudication
//! look was suppressed and the trap never ran. GNU and the pre-PR
//! base both fire exactly once for these shapes: the failure reaches
//! the time branch in its unconverted shape (absorbed completion
//! with status 1, or the propagated IoError) and the branch
//! adjudicates it.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn rubash_bin() -> String {
    env!("CARGO_BIN_EXE_rubash").to_string()
}

/// Run a script file; return (normalized stdout, normalized stderr, code).
fn run_script(tag: &str, body: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("issue2659-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    let path: PathBuf = dir.join("case.sh");
    fs::write(&path, body).expect("write script");
    let output = Command::new(rubash_bin())
        .arg(&path)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("run rubash");
    let normalize = |bytes: &[u8]| {
        String::from_utf8_lossy(bytes).replace(path.to_str().expect("utf8 path"), "S")
    };
    (
        normalize(&output.stdout),
        normalize(&output.stderr),
        output.status.code(),
    )
}

#[test]
fn issue2659_canonical_for_loop_redirect_fires_err_then_errexit() {
    // The issue's canonical shape: ERR trap fires, then the subshell
    // exits under set -e (no "after" line), subshell status 1.
    let (stdout, stderr, code) = run_script(
        "canon",
        "( set -e\n  trap 'echo got error ERR' ERR\n  for f in 1 2; do x=4; done < /no/such/dir/f\n  echo \"after: $?\" )\necho outer_rc=$?\n",
    );
    assert_eq!(stdout, "got error ERR\nouter_rc=1\n");
    // The redirect diagnostic itself is asserted loosely: GNU
    // attributes it to line 4 of this script, rubash to line 3 (the
    // for command's own line) — a line-attribution observation
    // recorded in the lane REPORT, not part of this issue's claim.
    assert!(
        stderr.contains("/no/such/dir/f: No such file or directory"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue2659_errexit_consequence_without_err_trap() {
    // set -e alone: the redirect failure must still exit the subshell.
    let (stdout, _stderr, code) = run_script(
        "errexit-only",
        "( set -e\n  for f in 1 2; do x=4; done < /no/such/dir/f\n  echo \"after: $?\" )\necho outer_rc=$?\n",
    );
    assert_eq!(stdout, "outer_rc=1\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue2659_brace_group_redirect_fires_err_and_continues() {
    // No set -e: the trap fires, the failure status is 1, execution
    // continues with the next command.
    let (stdout, _stderr, code) = run_script(
        "group",
        "trap 'echo gotERR' ERR\n{ echo hi; } < /no/such/dir/f\necho \"after: $?\"\n",
    );
    assert_eq!(stdout, "gotERR\nafter: 1\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue2659_while_loop_redirect_fires_err_and_continues() {
    let (stdout, _stderr, code) = run_script(
        "while",
        "trap 'echo gotERR' ERR\nwhile false; do :; done < /no/such/dir/f\necho \"after: $?\"\n",
    );
    assert_eq!(stdout, "gotERR\nafter: 1\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue2659_if_compound_redirect_fires_err_and_continues() {
    // The redirect belongs to the if command itself (applied before
    // the condition runs): the body never runs, the trap fires.
    let (stdout, _stderr, code) = run_script(
        "if",
        "trap 'echo gotERR' ERR\nif true; then echo body; fi < /no/such/dir/f\necho \"after: $?\"\n",
    );
    assert_eq!(stdout, "gotERR\nafter: 1\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue2659_loop_body_never_runs_and_trap_state_chains() {
    // Chain: the trap's side effect and the loop counter are both
    // observable afterwards — the body never executed (the redirect
    // failed before iteration 1) and the trap ran exactly once.
    let (stdout, _stderr, code) = run_script(
        "chain",
        "n=0; hits=0\ntrap 'hits=$((hits+1))' ERR\nfor f in 1 2 3; do n=$((n+1)); done < /no/such/dir/f\necho \"n=$n hits=$hits rc=$?\"\n",
    );
    assert_eq!(stdout, "n=0 hits=1 rc=1\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue2659_subshell_redirect_fires_err_and_continues() {
    let (stdout, _stderr, code) = run_script(
        "subshell",
        "trap 'echo gotERR' ERR\n( echo inner ) < /no/such/dir/f\necho \"after: $?\"\n",
    );
    assert_eq!(stdout, "gotERR\nafter: 1\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue2659_case_redirect_fires_err_and_continues() {
    let (stdout, _stderr, code) = run_script(
        "case",
        "trap 'echo gotERR' ERR\ncase x in x) echo cbody;; esac < /no/such/dir/f\necho \"after: $?\"\n",
    );
    assert_eq!(stdout, "gotERR\nafter: 1\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue2659_simple_command_redirect_still_fires_err() {
    // Regression lock: the simple-command redirect channel already
    // adjudicated the ERR trap before this fix (execute_cmd.c:993
    // face) and must keep doing so.
    let (stdout, _stderr, code) = run_script(
        "simple",
        "trap 'echo gotERR' ERR\necho hi < /no/such/dir/f\necho \"after: $?\"\n",
    );
    assert_eq!(stdout, "gotERR\nafter: 1\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue2659_timed_if_redirect_fires_err_and_continues() {
    // PR #3269 review R1: the timed compound's OWN redirect failure
    // is adjudicated by the ast_exec time branch once the timed
    // command completes — the trap fires, the status is 1, and the
    // redirect diagnostic is printed exactly once (the timing report
    // also lands on stderr but carries no path text).
    let (stdout, stderr, code) = run_script(
        "timed-if",
        "trap 'echo gotERR' ERR\ntime if true; then echo body; fi < /no/such/dir/f\necho \"after: $?\"\n",
    );
    assert_eq!(stdout, "gotERR\nafter: 1\n");
    assert_eq!(
        stderr
            .matches("/no/such/dir/f: No such file or directory")
            .count(),
        1,
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue2659_timed_for_redirect_fires_err_and_continues() {
    let (stdout, stderr, code) = run_script(
        "timed-for",
        "trap 'echo gotERR' ERR\ntime for f in 1 2; do x=4; done < /no/such/dir/f\necho \"after: $?\"\n",
    );
    assert_eq!(stdout, "gotERR\nafter: 1\n");
    assert_eq!(
        stderr
            .matches("/no/such/dir/f: No such file or directory")
            .count(),
        1,
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue2659_timed_while_redirect_fires_err_and_continues() {
    let (stdout, _stderr, code) = run_script(
        "timed-while",
        "trap 'echo gotERR' ERR\ntime while false; do :; done < /no/such/dir/f\necho \"after: $?\"\n",
    );
    assert_eq!(stdout, "gotERR\nafter: 1\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue2659_timed_brace_redirect_fires_err_and_continues() {
    let (stdout, stderr, code) = run_script(
        "timed-brace",
        "trap 'echo gotERR' ERR\ntime { echo hi; } < /no/such/dir/f\necho \"after: $?\"\n",
    );
    assert_eq!(stdout, "gotERR\nafter: 1\n");
    assert_eq!(
        stderr
            .matches("/no/such/dir/f: No such file or directory")
            .count(),
        1,
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue2659_timed_errexit_fires_err_then_exits() {
    // set -e + timed for: GNU fires the ERR trap first, then the
    // errexit consequence exits the subshell with status 1 (the
    // "after" line inside the subshell never prints).
    let (stdout, _stderr, code) = run_script(
        "timed-errexit",
        "( set -e; trap 'echo gotERR' ERR; time for f in 1 2; do x=4; done < /no/such/dir/f; echo after )\necho outer=$?\n",
    );
    assert_eq!(stdout, "gotERR\nouter=1\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue2659_timed_brace_wrapped_compound_failure_fires_err_once() {
    // No-double-fire proof: the inner if's own redirect failure is a
    // candidate at the absorption site AND at the time branch (the
    // timed brace completes with the failure status). Exactly one
    // fire, and the timed command's status is the failure's 1.
    let (stdout, _stderr, code) = run_script(
        "timed-brace-wrap",
        "trap 'echo gotERR' ERR\ntime { if true; then echo body; fi < /no/such/dir/f; }\necho \"after: $?\"\n",
    );
    assert_eq!(stdout, "gotERR\nafter: 1\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue2659_timed_time_chain_fires_err_once() {
    // No-double-fire proof for nested time frames: two time branches
    // wrap one redirect failure; the trap fires exactly once.
    let (stdout, _stderr, code) = run_script(
        "timed-chain",
        "trap 'echo gotERR' ERR\ntime time if true; then echo body; fi < /no/such/dir/f\necho \"after: $?\"\n",
    );
    assert_eq!(stdout, "gotERR\nafter: 1\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue2659_timed_simple_redirect_still_fires_err() {
    // Regression lock: timed SIMPLE commands already adjudicated the
    // ERR trap correctly before the R1 rework (the dispatch-level
    // timed gates cover them) and must keep doing so.
    let (stdout, _stderr, code) = run_script(
        "timed-simple",
        "trap 'echo gotERR' ERR\ntime echo hi < /no/such/dir/f\necho \"after: $?\"\n",
    );
    assert_eq!(stdout, "gotERR\nafter: 1\n");
    assert_eq!(code, Some(0));
}
