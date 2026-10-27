//! rubash#1832: when an `env` command was the exec-replace TAIL of a
//! `-c` string, the #1497 seam stamped
//! `process.env("SHLVL", exec_replace_adjusted_shlvl())` AFTER the env
//! emulation had composed the final child environment — resurrecting
//! a variable the composition deliberately omitted (`env -i`),
//! removed (`env -u SHLVL`), or set (`env SHLVL=99`).
//!
//! GNU: the adjustment is a mutation of the shell VARIABLE before the
//! export environment is built — execute_cmd.c:5813-5817
//! (execute_disk_command: on the nofork path, adjust_shell_level(-1)
//! runs first, then maybe_make_export_env()). The environment `env`
//! RECEIVES therefore already carries the adjusted level, and env's
//! own `-i`/`-u`/assignment processing governs what the final program
//! sees. The composition is the last hand; no post-composition stamp
//! exists anywhere in GNU's chain.
//!
//! Oracle: GNU bash 5.3.20 with coreutils env 9.4, purified
//! environment, script file / `-c` string per case below.

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

fn shlvl_lines(stdout: &str) -> Vec<&str> {
    stdout.lines().filter(|l| l.starts_with("SHLVL=")).collect()
}

#[test]
fn issue1832_env_i_tail_yields_exactly_the_given_assignments() {
    if !env_binary() {
        return;
    }
    // The issue's seed shape, no inherited SHLVL: GNU prints only A=1;
    // the seam used to append SHLVL=0.
    let (stdout, stderr, code) = rubash("env -i A=1 /usr/bin/env", &[]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "A=1\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1832_env_i_tail_with_inherited_shlvl() {
    if !env_binary() {
        return;
    }
    // Same shape with SHLVL inherited: `-i` discards the environment
    // env received (adjusted or not) wholesale.
    let (stdout, stderr, code) = rubash("env -i A=1 /usr/bin/env", &[("SHLVL", "5")]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "A=1\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1832_env_unset_shlvl_tail_stays_unset() {
    if !env_binary() {
        return;
    }
    let (stdout, stderr, code) = rubash("env -u SHLVL /usr/bin/env", &[("SHLVL", "5")]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(shlvl_lines(&stdout), Vec::<&str>::new());
    assert_eq!(stderr, "");
}

#[test]
fn issue1832_env_explicit_shlvl_assignment_tail_wins() {
    if !env_binary() {
        return;
    }
    // An explicit env-side assignment is env's own processing of the
    // received (adjusted) environment: it supersedes the adjustment.
    let (stdout, stderr, code) = rubash("env SHLVL=99 /usr/bin/env", &[("SHLVL", "5")]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(shlvl_lines(&stdout), vec!["SHLVL=99"]);
    assert_eq!(stderr, "");
}

#[test]
fn issue1832_env_passthrough_tail_carries_adjusted_level() {
    if !env_binary() {
        return;
    }
    // No -i/-u/assignment: env passes its received environment through
    // — the ADJUSTED level (inherited 5, shell ran at 6, tail exec
    // decrements back to 5), not the shell's live 6.
    let (stdout, stderr, code) = rubash("env /usr/bin/env", &[("SHLVL", "5")]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(shlvl_lines(&stdout), vec!["SHLVL=5"]);
    assert_eq!(stderr, "");
}

#[test]
fn issue1832_env_print_tail_shows_adjusted_level() {
    if !env_binary() {
        return;
    }
    // The bare-`env` print path emits the composed environment env
    // received — again the adjusted level.
    let (stdout, stderr, code) = rubash("env", &[("SHLVL", "5")]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(shlvl_lines(&stdout), vec!["SHLVL=5"]);
    assert_eq!(stderr, "");
}

#[test]
fn issue1832_prefix_shlvl_on_env_tail_loses_to_adjustment() {
    if !env_binary() {
        return;
    }
    // A prefix (tempenv) SHLVL word on the tail command itself is
    // overridden by the adjustment, exactly as on the direct path
    // (oracle: `SHLVL=99 env /usr/bin/env` prints SHLVL=5).
    let (stdout, stderr, code) = rubash("SHLVL=99 env /usr/bin/env", &[("SHLVL", "5")]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(shlvl_lines(&stdout), vec!["SHLVL=5"]);
    assert_eq!(stderr, "");
}

#[test]
fn issue1832_direct_tail_control_keeps_adjusted_level() {
    if !env_binary() {
        return;
    }
    // Control: the direct (non-env) tail keeps the #1497 behavior.
    let (stdout, stderr, code) = rubash("/usr/bin/env", &[("SHLVL", "5")]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(shlvl_lines(&stdout), vec!["SHLVL=5"]);
    assert_eq!(stderr, "");
}

#[test]
fn issue1832_pipeline_control_keeps_unadjusted_level() {
    if !env_binary() {
        return;
    }
    // Control: a pipeline element is forked, never adjusted — the
    // child sees the shell's live level (inherited 5 + 1).
    let (stdout, stderr, code) = rubash("/usr/bin/env | cat", &[("SHLVL", "5")]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(shlvl_lines(&stdout), vec!["SHLVL=6"]);
    assert_eq!(stderr, "");
}

#[test]
fn issue1832_non_tail_env_i_control() {
    if !env_binary() {
        return;
    }
    // Control: the #1806 composition contract in non-tail position is
    // untouched by the seam fix.
    let (stdout, stderr, code) = rubash("env -i A=1 /usr/bin/env; true", &[("SHLVL", "5")]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "A=1\n");
    assert_eq!(stderr, "");
}
