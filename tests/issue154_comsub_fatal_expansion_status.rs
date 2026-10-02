//! Issue niubash#154 (rescoped comsub corner), rubash lane wt32/optfix:
//! in `-c` mode a fatal parameter expansion INSIDE a `$( )` command
//! substitution made the comsub/assignment report 127 (rubash) where WSL
//! GNU Bash 5.3.0 reports 1.
//!
//! GNU spec (subst.c command_substitute child exit path): the comsub body
//! runs through parse_and_execute in the forked child, whose top_level
//! setjmp (subst.c:7393-7404) turns the fatal word-expansion longjmp
//! (expand_wdesc_fatal → exp_jump_to_top_level) into
//! exit(EXECUTION_FAILURE) — 1 — via last_command_exit_value/rc
//! (subst.c:7415-7418). The 127 mapping lives only in shell.c:1471
//! run_one_command, the `-c` TOP-LEVEL driver, which the comsub child
//! never runs. Direct `-c` fatals (no comsub) still exit 127.
//!
//! Diagnostic behavior was re-measured for this fix and is the OPPOSITE
//! of the issue's rescope note: GNU does NOT swallow the `${x:?}` line
//! inside a comsub — parameter_brace_expand_error (subst.c:8216)
//! report_error's it unconditionally and the child inherits stderr
//! (subst.c:7306 dups only fd 1). Separated-stream script-file probes:
//! stderr carries `bash: line 1: x: boom`; rubash matches byte-for-byte,
//! so no swallow is implemented (it would diverge from GNU).
//!
//! Rust semantic owner: command_substitution.rs
//! command_list_substitution_output_typed sets __RUBASH_COMSUB_BODY on the
//! body executor (the fork-model $( )/backtick path only — the nofork
//! funsub/valsub keeps the -c 127 propagation, GNU subst.c:7057
//! exp_jump_to_top_level), and parameter_errors.rs expansion_fatal_status
//! returns 1 under that flag.

#![cfg(windows)]

use std::io::Write;
use std::process::{Command, Stdio};

struct RunOutcome {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_rubash_c(command: &str) -> RunOutcome {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", command])
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash -c");
    RunOutcome {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn run_rubash_script(script: &str) -> RunOutcome {
    let dir = std::env::temp_dir().join(format!("rubash-issue154-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue154 scratch dir");
    let script_path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&script_path).expect("write probe script");
    file.write_all(script.as_bytes())
        .expect("write probe script");
    drop(file);
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash script");
    let _ = std::fs::remove_dir_all(&dir);
    RunOutcome {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// rubash#154 shape (a): `bash -c 'v=$(echo ${x:?boom})'` — the comsub
/// child exits EXECUTION_FAILURE (1); the session's last command (the
/// assignment) carries it. GNU-measured rc 1, stderr `bash: line 1:
/// x: boom` (diagnostic NOT swallowed).
#[test]
fn c_mode_comsub_fatal_last_command_exits_one() {
    let out = run_rubash_c("v=$(echo ${x:?boom})");
    assert_eq!(out.code, Some(1));
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr, "bash: line 1: x: boom\n");
}

/// The session continues past a failed comsub assignment: the assignment
/// status is the comsub's 1 (GNU matrix F row: `after:1:[]`, rc 0).
#[test]
fn c_mode_comsub_fatal_continues_with_status_one() {
    let out = run_rubash_c("v=$(echo ${x:?boom}); echo \"after:$?:[$v]\"");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "after:1:[]\n");
    assert_eq!(out.stderr, "bash: line 1: x: boom\n");
}

/// nounset flavor: `set -u; v=$(echo $UNDEF)` — comsub status 1.
#[test]
fn c_mode_nounset_inside_comsub_reports_one() {
    let out = run_rubash_c("set -u; v=$(echo $UNDEF_INNER)");
    assert_eq!(out.code, Some(1));
    assert_eq!(out.stderr, "bash: line 1: UNDEF_INNER: unbound variable\n");
}

/// Protected main path (GNU-correct, must NOT change): direct `-c`
/// fatals without a comsub still exit 127 (shell.c:1471 FORCE_EOF).
#[test]
fn c_mode_direct_fatals_stay_127() {
    let out = run_rubash_c("v=${x:?direct}");
    assert_eq!(out.code, Some(127));

    let out = run_rubash_c("set -u; v=$UNDEF_DIRECT");
    assert_eq!(out.code, Some(127));

    // Function body without a comsub: still 127.
    let out = run_rubash_c("f(){ echo ${x:?fnbody}; }; f; echo after");
    assert_eq!(out.code, Some(127));
    assert_eq!(out.stdout, "");
}

/// Guards: the flag must not touch genuine child statuses.
#[test]
fn comsub_child_statuses_are_untouched() {
    let out = run_rubash_c("v=$(exit 127); echo \"rc=$?\"");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "rc=127\n");

    let out = run_rubash_c("v=$(nosuchcmd_xyz_123)");
    assert_eq!(out.code, Some(127));

    let out = run_rubash_c("v=$(false); echo \"rc=$?\"");
    assert_eq!(out.stdout, "rc=1\n");
}

/// Function frame around the comsub: the comsub failure is contained
/// (infn:1, afterfn:0 — GNU matrix K row).
#[test]
fn c_mode_function_wrapping_comsub_is_contained() {
    let out =
        run_rubash_c("f(){ v=$(echo ${x:?infn}); echo \"infn:$?\"; }; f; echo \"afterfn:$?\"");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "infn:1\nafterfn:0\n");
}

/// The nofork funsub `${ ...; }` keeps the -c 127 propagation (GNU
/// subst.c:7057 exp_jump_to_top_level out of the in-parent body).
#[test]
fn c_mode_funsub_fatal_stays_127() {
    let out = run_rubash_c("v=${ echo ${x:?fs}; }; echo after:$?");
    assert_eq!(out.code, Some(127));
    assert_eq!(out.stdout, "");
}

/// Script mode (already GNU-correct before this fix — regression guard):
/// comsub status 1, script continues, diagnostic carries the script
/// prolog.
#[test]
fn script_mode_comsub_fatal_reports_one_and_continues() {
    let out = run_rubash_script(concat!(
        "v=$(echo ${x:?boom})\n",
        "echo \"after:$?:[$v]\"\n",
        "set -u\n",
        "v3=$(echo $UNDEF_INNER)\n",
        "echo \"after3:$?\"\n",
    ));
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "after:1:[]\nafter3:1\n");
    // The diagnostic prolog carries the full script path; assert by parts.
    assert!(out.stderr.contains("probe.sh: line 1: x: boom\n"));
    assert!(out
        .stderr
        .contains("probe.sh: line 4: UNDEF_INNER: unbound variable\n"));
    assert_eq!(out.stderr.lines().count(), 2);
}
