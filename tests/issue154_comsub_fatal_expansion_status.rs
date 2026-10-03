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

// ---------------------------------------------------------------------------
// niubash#163 lane wt55/subshell-rc: the `-c` subshell exit-status cell.
//
// GNU spec: a fatal expansion error raises FORCE_EOF with
// last_command_exit_value = EXECUTION_FAILURE (subst.c:10168/11027 nounset,
// subst.c:8158 `:?`, expr.c:1190-1216 `(( ))`/arith nounset). The 127 remap
// exists ONLY at shell.c:1471 run_one_command — the `-c` top-level catch.
// A `( ... )` subshell child re-arms top_level at execute_cmd.c:1811
// (execute_in_subshell) and converts any jump to last_command_exit_value
// (1), so `bash -c '( set -u; echo $U )'` exits 1, while the same fatal at
// `-c` top level, in a function, or in a `for` body still exits 127. The
// comsub child does the same at subst.c:7393-7404 (rubash#154 above).
//
// Rust semantic owner: parameter_errors.rs expansion_fatal_status — the
// single context owner (subshell_depth > 0 / __RUBASH_COMSUB_BODY → 1,
// else __RUBASH_IS_C → 127, else 1). Every matrix row below was measured
// against WSL GNU Bash 5.3.0 (/usr/local/bin/bash) on 2026-10-02, both
// `-c` and script-file mode.
// ---------------------------------------------------------------------------

/// Run `rubash -c` with #163 probe hygiene (U_X scrubbed from the
/// environment so `set -u` fatality is deterministic).
fn run163_c(command: &str) -> RunOutcome {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", command])
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .env_remove("U_X")
        .output()
        .expect("run rubash -c");
    RunOutcome {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Script-file mode counterpart of [`run163_c`].
fn run163_file(script: &str) -> RunOutcome {
    let dir = std::env::temp_dir().join(format!("rubash-issue163-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue163 scratch dir");
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
        .env_remove("U_X")
        .output()
        .expect("run rubash script");
    let _ = std::fs::remove_dir_all(&dir);
    RunOutcome {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// The niubash#163 seven-shape matrix × two modes, rc parity with GNU:
/// (shape, -c rc, script-file rc). The subshell rows are the #163 fix
/// (were 127 under `-c`); the 127 rows are the protected top-level /
/// function / for shapes that must NOT regress to 1.
#[test]
fn issue163_matrix_c_and_file_mode_exit_parity() {
    // (shape, expected -c rc, expected file rc) — GNU-measured.
    let matrix: &[(&str, i32, i32)] = &[
        // #163 headline shapes: subshell-contained fatals exit 1.
        ("( set -u; echo \"$U_X\" )", 1, 1),
        ("( echo \"${U_X:?boom}\" )", 1, 1),
        // Protected 127 shapes: fatal reaches run_one_command's catch.
        ("set -u; echo \"$U_X\"", 127, 1),
        ("f(){ set -u; echo \"$U_X\"; }; f", 127, 1),
        ("for i in 1; do set -u; echo \"$U_X\"; done", 127, 1),
        // Comsub child (rubash#154) and continuation shapes.
        ("x=$( set -u; echo \"$U_X\" )", 1, 1),
        ("( set -u; echo \"$U_X\" ); :", 0, 0),
        // The same containment for the arithmetic-nounset and posix-fatal
        // FORCE_EOF raisers (expr.c:1190, subst.c:4295) inside `( )`.
        ("( set -u; echo $((U_X)) )", 1, 1),
        ("( ( set -u; echo \"$U_X\" ) )", 1, 1),
        ("( set -o posix; echo $((1/0)) )", 1, 1),
        ("( set -u; v=$((U_X)) )", 1, 1),
        // Top-level arithmetic nounset keeps the `-c` 127 (and is 1 in
        // script mode — eval.c:104-109 exits last_command_exit_value).
        ("set -u; echo $((U_X))", 127, 1),
        ("set -u; ((U_X))", 127, 1),
        ("set -u; v=$((U_X))", 127, 1),
        ("set -o posix; echo $((1/0))", 127, 1),
    ];
    for (shape, c_rc, file_rc) in matrix {
        let out = run163_c(shape);
        assert_eq!(out.code, Some(*c_rc), "-c rc mismatch for: {shape}");
        let out = run163_file(&format!("{shape}\n"));
        assert_eq!(out.code, Some(*file_rc), "file rc mismatch for: {shape}");
    }
}

/// The subshell boundary CONTAINS the fatal (the script continues with
/// $? = 1) — GNU `bash -c '( echo "${U_X:?boom}" ); echo after=$?'`
/// prints `after=1`, rc 0.
#[test]
fn issue163_subshell_fatal_is_contained_with_status_one() {
    let out = run163_c("( echo \"${U_X:?boom}\" ); echo after=$?");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "after=1\n");
    assert_eq!(out.stderr, "bash: line 1: U_X: boom\n");

    let out = run163_file("( set -u; echo \"$U_X\" ); echo after=$?\n");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "after=1\n");
}

/// GNU-measured diagnostics for the two #163 headline shapes (stderr text
/// is part of the parity contract).
#[test]
fn issue163_subshell_fatal_diagnostics_match() {
    let out = run163_c("( set -u; echo \"$U_X\" )");
    assert_eq!(out.stderr, "bash: line 1: U_X: unbound variable\n");
    assert_eq!(out.stdout, "");

    let out = run163_c("( echo \"${U_X:?boom}\" )");
    assert_eq!(out.stderr, "bash: line 1: U_X: boom\n");
}

/// An async paren subshell is a forked child that re-arms top_level: its
/// fatal exit is 1 (`wait` reports 1; GNU execute_cmd.c:1811). A plain
/// async command's fork does NOT re-arm, so under `-c` it inherits the
/// run_one_command catch and `wait` reports 127.
#[test]
fn issue163_async_paren_subshell_waits_with_one() {
    let out = run163_c("( echo \"${U_X:?}\" ) & wait $!; echo bg=$?");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "bg=1\n");

    let out = run163_c("set -u; echo \"$U_X\" & wait $!; echo bg=$?");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "bg=127\n");
}

/// The nofork funsub boundary is NOT a subshell: it keeps the `-c` 127
/// even though it looks like a substitution (GNU subst.c:7057 runs the
/// body in-parent). Companion guard for the pinned test above.
#[test]
fn issue163_funsub_inside_subshell_is_contained_but_direct_stays_127() {
    // Direct funsub fatal: 127 (protected above as
    // c_mode_funsub_fatal_stays_127). Inside `( )` the funsub's fatal
    // dies at the subshell boundary: 1.
    let out = run163_c("( v=${ echo ${x:?fs}; }; echo after )");
    assert_eq!(out.code, Some(1));
    assert_eq!(out.stdout, "");
}
