//! rubash#3310: `exec` replaces the shell, so GNU decrements SHLVL by 1
//! before the exec (builtins/exec.def:193-201, variables.c:848-890
//! `adjust_shell_level(-1)`); rubash passed SHLVL through unchanged.
//!
//! GNU (vendored bash 5.3.20, script-file probes):
//! - `SHLVL=5 exec /bin/sh -c 'echo $SHLVL'` -> 4 (the issue seed).
//! - `SHLVL=5 exec /bin/bash -c 'echo $SHLVL'` -> 5 (4, then the new
//!   bash's startup increment brings it back to 5).
//! - SHLVL=0 stays 0; absent / unset / non-numeric SHLVL yields 0
//!   (adjust_shell_level counts those as 0 and binds the variable).
//! - `exec -c` passes no SHLVL at all (cleanenv skips the adjustment).
//! - A prefix assignment IS visible to the adjustment
//!   (`SHLVL=9 exec ...` -> 8), because the temps are applied before
//!   the special builtin runs.
//! - `( SHLVL=5 exec ... )` keeps 5: exec.def skips the decrement in
//!   a `( )` subshell (SUBSHELL_PAREN).
//! - `$( SHLVL=5 exec ... )` yields 4: a command substitution is
//!   SUBSHELL_COMSUB, not PAREN, so the decrement applies.
//! - A plain spawn (`SHLVL=5 /bin/sh ...`, no exec) is unchanged.
//! - An unexported SHLVL is still adjusted and exported to the child
//!   (adjust_shell_level binds + auto-exports).
//! - The internal `exec printenv` emulation reports the adjusted level
//!   too (it stands in for the exec'd image): `SHLVL=5 exec printenv
//!   SHLVL` -> 4, and an unexported/absent SHLVL appears with the
//!   adjusted value.
//!
//! Harness note: the shell under test starts with the inherited SHLVL
//! plus one (initialize_shell_level), so extra_env SHLVL=5 means the
//! shell runs live at 6: a bare `exec` yields 5, a `SHLVL=5` prefix
//! yields 4.
//!
//! Not nailed here: the pipeline-element face (`SHLVL=5 exec ... |
//! cat` -> GNU 4). The paren latch this fix reads
//! (subshell_login_cleared) is also set for pipeline elements, so
//! that face still skips the decrement; see REPORT.md — it needs a
//! real stage marker, not an fd-shape heuristic. The >= 1000
//! warning-and-reset-to-1 arm of adjust_shell_level is likewise not
//! reproduced (absent from every sibling port too); see REPORT.md.

#![cfg(unix)]

use std::process::Command;

fn rubash(script: &str, extra_env: &[(&str, &str)]) -> (String, String, Option<i32>) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .arg("-c")
        .arg(script)
        .env_clear()
        .env("PATH", "/usr/bin:/bin");
    for (name, value) in extra_env {
        command.env(name, value);
    }
    let output = command.output().expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn env_binary() -> bool {
    std::path::Path::new("/usr/bin/env").is_file()
}

fn bash_binary() -> bool {
    std::path::Path::new("/bin/bash").is_file()
}

fn shlvl_lines(stdout: &str) -> Vec<&str> {
    stdout.lines().filter(|l| l.starts_with("SHLVL=")).collect()
}

#[test]
fn issue3310_seed_prefix_five_exec_sh_yields_four() {
    // The issue's seed shape.
    let (stdout, stderr, code) = rubash("SHLVL=5 exec /bin/sh -c 'echo $SHLVL'", &[("SHLVL", "5")]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "4\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue3310_top_exec_env_yields_live_minus_one() {
    if !env_binary() {
        return;
    }
    // No prefix: the live level is 6 (inherited 5 + 1); exec takes it
    // back to 5.
    let (stdout, stderr, code) = rubash("exec /usr/bin/env", &[("SHLVL", "5")]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(shlvl_lines(&stdout), vec!["SHLVL=5"]);
    assert_eq!(stderr, "");
}

#[test]
fn issue3310_top_exec_bash_yields_live() {
    if !bash_binary() {
        return;
    }
    // Issue's second shape: 5 -> 4, then the new bash's startup
    // increment restores 5.
    let (stdout, stderr, code) =
        rubash("SHLVL=5 exec /bin/bash -c 'echo $SHLVL'", &[("SHLVL", "5")]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "5\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue3310_zero_stays_zero() {
    let (stdout, stderr, code) = rubash("SHLVL=0 exec /bin/sh -c 'echo $SHLVL'", &[("SHLVL", "5")]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "0\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue3310_absent_shlvl_exec_yields_zero() {
    if !env_binary() {
        return;
    }
    // No SHLVL inherited: the shell starts at 1, exec binds 0.
    let (stdout, stderr, code) = rubash("exec /usr/bin/env", &[]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(shlvl_lines(&stdout), vec!["SHLVL=0"]);
    assert_eq!(stderr, "");
}

#[test]
fn issue3310_unset_shlvl_exec_yields_zero() {
    if !env_binary() {
        return;
    }
    let (stdout, stderr, code) = rubash("unset SHLVL; exec /usr/bin/env", &[("SHLVL", "5")]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(shlvl_lines(&stdout), vec!["SHLVL=0"]);
    assert_eq!(stderr, "");
}

#[test]
fn issue3310_non_numeric_shlvl_exec_yields_zero() {
    if !env_binary() {
        return;
    }
    // GNU counts an unparseable SHLVL as 0.
    let (stdout, stderr, code) = rubash("SHLVL=abc exec /usr/bin/env", &[("SHLVL", "5")]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(shlvl_lines(&stdout), vec!["SHLVL=0"]);
    assert_eq!(stderr, "");
}

#[test]
fn issue3310_exec_dash_c_passes_no_shlvl() {
    if !env_binary() {
        return;
    }
    // exec -c clears the environment wholesale; no SHLVL crosses.
    let (stdout, stderr, code) = rubash("SHLVL=5 exec -c /usr/bin/env", &[("SHLVL", "5")]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(shlvl_lines(&stdout), Vec::<&str>::new());
    assert_eq!(stderr, "");
}

#[test]
fn issue3310_paren_exec_keeps_level() {
    // SUBSHELL_PAREN: no decrement inside `( )`.
    let (stdout, stderr, code) =
        rubash("(SHLVL=5 exec /bin/sh -c 'echo $SHLVL')", &[("SHLVL", "5")]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "5\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue3310_paren_exec_env_keeps_live_level() {
    if !env_binary() {
        return;
    }
    let (stdout, stderr, code) = rubash("(exec /usr/bin/env)", &[("SHLVL", "5")]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(shlvl_lines(&stdout), vec!["SHLVL=6"]);
    assert_eq!(stderr, "");
}

#[test]
fn issue3310_comsub_exec_decrements() {
    // SUBSHELL_COMSUB is not PAREN: the decrement applies.
    let (stdout, stderr, code) = rubash(
        "echo $(SHLVL=5 exec /bin/sh -c 'echo $SHLVL')",
        &[("SHLVL", "5")],
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "4\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue3310_plain_spawn_keeps_level() {
    // Control: no exec, no decrement.
    let (stdout, stderr, code) = rubash("SHLVL=5 /bin/sh -c 'echo $SHLVL'", &[("SHLVL", "5")]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "5\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue3310_plain_spawn_env_keeps_live_level() {
    if !env_binary() {
        return;
    }
    // GNU 5.3.20 oracle (harness shape: `-c`, env_clear + SHLVL=5):
    // a plain-spawned child sees SHLVL=5, NOT 6 — the live level is 6
    // but the exported level under `-c` is 5. The script-file form does
    // export 6 (`SHLVL=5 bash file` -> child 6); the earlier `6`
    // expectation was copied from that form. The sibling
    // `plain_spawn_keeps_level` pin already asserts 5 for the same
    // `-c` shape, and the product matches GNU here — pin-only fix.
    let (stdout, stderr, code) = rubash("/usr/bin/env", &[("SHLVL", "5")]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(shlvl_lines(&stdout), vec!["SHLVL=5"]);
    assert_eq!(stderr, "");
}

#[test]
fn issue3310_brace_group_exec_decrements() {
    // `{ ...; }` is not a subshell: the decrement applies.
    let (stdout, stderr, code) = rubash(
        "{ SHLVL=5 exec /bin/sh -c 'echo $SHLVL'; }",
        &[("SHLVL", "5")],
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "4\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue3310_unexported_shlvl_exec_still_adjusts() {
    if !env_binary() {
        return;
    }
    // adjust_shell_level auto-exports: the child sees the adjusted
    // level even though the shell variable was unexported.
    let (stdout, stderr, code) = rubash("export -n SHLVL; exec /usr/bin/env", &[("SHLVL", "5")]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(shlvl_lines(&stdout), vec!["SHLVL=5"]);
    assert_eq!(stderr, "");
}

#[test]
fn issue3310_prefix_nine_exec_yields_eight() {
    // GNU c9: the prefix assignment is visible to the adjustment.
    let (stdout, stderr, code) = rubash("SHLVL=9 exec /bin/sh -c 'echo $SHLVL'", &[("SHLVL", "5")]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "8\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue3310_exec_printenv_shlvl_yields_adjusted() {
    // The internal printenv emulation stands in for the exec'd image,
    // so it reports the adjusted level too (GNU: 4).
    let (stdout, stderr, code) = rubash("SHLVL=5 exec printenv SHLVL", &[("SHLVL", "5")]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "4\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue3310_exec_printenv_all_shows_adjusted() {
    let (stdout, stderr, code) = rubash("SHLVL=5 exec printenv", &[("SHLVL", "5")]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(shlvl_lines(&stdout), vec!["SHLVL=4"]);
    assert_eq!(stderr, "");
}

#[test]
fn issue3310_exec_printenv_unexported_shows_adjusted() {
    // adjust_shell_level auto-exports: the emulation lists SHLVL
    // even though the shell variable was unexported.
    let (stdout, stderr, code) = rubash("export -n SHLVL; exec printenv SHLVL", &[("SHLVL", "5")]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "5\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue3310_exec_printenv_dash_c_empty() {
    let (stdout, stderr, code) = rubash("SHLVL=5 exec -c printenv SHLVL", &[("SHLVL", "5")]);
    assert_eq!(code, Some(1), "stderr: {stderr}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
}

#[test]
fn issue3310_paren_exec_printenv_keeps_level() {
    let (stdout, stderr, code) = rubash("(SHLVL=5 exec printenv SHLVL)", &[("SHLVL", "5")]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "5\n");
    assert_eq!(stderr, "");
}
