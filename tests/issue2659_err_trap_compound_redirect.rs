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
        String::from_utf8_lossy(bytes)
            .replace(path.to_str().expect("utf8 path"), "S")
            .into_owned()
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
    assert_eq!(
        stderr,
        "S: line 3: /no/such/dir/f: No such file or directory\n"
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
