//! Issue rubash#1806: the environment composed for a command carrying
//! prefix assignments diverged from GNU in composition and content —
//! rubash printed the prefix bindings LAST (after the globals), leaked
//! its fabricated THIS_SH into the exported set, and never set `_`.
//!
//! GNU model (variables.c maybe_make_export_env): the child env array
//! is flattened in block order — the temporary_env context (the
//! command's prefix assignments) FIRST, then the shell_variables
//! contexts, then the exported functions — under add_or_supercede
//! semantics (a name already present is replaced in place). Finally
//! execute_cmd.c execute_disk_command calls put_command_name_into_env
//! (variables.c), setting `_` to the pathname search_for_command
//! resolved for the command about to run, superseding any inherited
//! or prefix-assigned entry; when `_` was itself prefix-assigned it
//! therefore stays in the prefix slot carrying the program path
//! (oracle: `_=foo env` prints `_=/usr/bin/env` FIRST). Within a block
//! GNU iterates hash chains — an artifact order, not a contract — so
//! these pins lock block composition, membership, and values; the
//! prefix block's internal order is rubash's deterministic choice
//! (source order), the globals block is composed sorted by name.
//!
//! Expected outputs verified against GNU bash 5.3.20 with coreutils
//! env 9.4 under a purified environment.

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

fn env_lines(script: &str) -> Vec<String> {
    let (stdout, stderr, code) = rubash(script, &[]);
    assert_eq!(code, Some(0), "script {script:?} stderr: {stderr}");
    assert_eq!(stderr, "", "script {script:?}");
    stdout.lines().map(str::to_string).collect()
}

fn env_binary() -> bool {
    std::path::Path::new("/usr/bin/env").is_file()
}

#[test]
fn issue1806_prefix_block_first_globals_middle_underscore_last() {
    if !env_binary() {
        return;
    }
    let lines = env_lines("x=1 env");
    // The prefix block opens the composition: x=1 is the first line.
    assert_eq!(lines.first().map(String::as_str), Some("x=1"), "{lines:?}");
    // `_` naming the resolved env program closes it.
    assert_eq!(
        lines.last().map(String::as_str),
        Some("_=/usr/bin/env"),
        "{lines:?}"
    );
    // The exported globals sit between the two, each exactly once.
    for name in ["PATH=", "PWD=", "SHLVL="] {
        assert_eq!(
            lines.iter().filter(|line| line.starts_with(name)).count(),
            1,
            "{name} in {lines:?}"
        );
    }
    assert_eq!(
        lines.iter().filter(|line| line.starts_with("x=")).count(),
        1
    );
    assert_eq!(
        lines.iter().filter(|line| line.starts_with("_=")).count(),
        1
    );
}

#[test]
fn issue1806_multiple_prefix_assignments_keep_source_order() {
    if !env_binary() {
        return;
    }
    let lines = env_lines("zz=1 aa=2 env");
    assert_eq!(lines.first().map(String::as_str), Some("zz=1"), "{lines:?}");
    assert_eq!(lines.get(1).map(String::as_str), Some("aa=2"), "{lines:?}");
    assert_eq!(
        lines.last().map(String::as_str),
        Some("_=/usr/bin/env"),
        "{lines:?}"
    );
}

#[test]
fn issue1806_underscore_supersedes_a_prefix_assigned_underscore() {
    if !env_binary() {
        return;
    }
    // GNU add_or_supercede: the prefix slot survives, its value is
    // replaced by the resolved program pathname.
    let lines = env_lines("_=foo env");
    assert_eq!(
        lines.first().map(String::as_str),
        Some("_=/usr/bin/env"),
        "{lines:?}"
    );
    assert!(!lines.iter().any(|line| line == "_=foo"), "{lines:?}");
}

#[test]
fn issue1806_fabricated_this_sh_is_a_shell_variable_not_an_export() {
    // Started without THIS_SH in the environment: the shell fabricates
    // it for ${THIS_SH} recursion but must not export the fabrication.
    let lines = env_lines("env");
    assert!(
        !lines.iter().any(|line| line.starts_with("THIS_SH=")),
        "{lines:?}"
    );
    // The variable itself still expands inside the shell.
    let (stdout, _, code) = rubash("echo \"sh=${THIS_SH:+set}\"", &[]);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "sh=set\n");
}

#[test]
fn issue1806_inherited_this_sh_keeps_its_export_mark() {
    // An inherited THIS_SH is an ordinary imported variable: GNU
    // passes imported marked variables through to children.
    let (stdout, stderr, code) = rubash("env", &[("THIS_SH", env!("CARGO_BIN_EXE_rubash"))]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.lines().any(|line| line.starts_with("THIS_SH=")),
        "{stdout}"
    );
}

#[test]
fn issue1806_spawned_children_receive_underscore_naming_the_program() {
    if !env_binary() || !std::path::Path::new("/usr/bin/printenv").is_file() {
        return;
    }
    // Real spawn path: `_` names the program actually executed.
    let lines = env_lines("/usr/bin/env");
    assert!(
        lines.iter().any(|line| line == "_=/usr/bin/env"),
        "{lines:?}"
    );
    // With a prefix assignment: both cross, `_` still names the program.
    let lines = env_lines("x=1 /usr/bin/env");
    assert!(lines.iter().any(|line| line == "x=1"), "{lines:?}");
    assert!(
        lines.iter().any(|line| line == "_=/usr/bin/env"),
        "{lines:?}"
    );
    assert!(
        !lines.iter().any(|line| line.starts_with("THIS_SH=")),
        "{lines:?}"
    );
    // A prefix-assigned `_` is superseded on the spawn path too.
    let lines = env_lines("_=foo /usr/bin/env");
    assert!(
        lines.iter().any(|line| line == "_=/usr/bin/env"),
        "{lines:?}"
    );
    assert!(!lines.iter().any(|line| line == "_=foo"), "{lines:?}");
    // Pipeline stage: each stage's `_` names its own program.
    let (stdout, _, code) = rubash("x=1 /usr/bin/env | grep '^_='", &[]);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "_=/usr/bin/env\n");
    // The `env` emulation hands its composed `_` (naming env itself)
    // to its subcommand untouched, exactly like coreutils env.
    let (stdout, _, code) = rubash("env printenv _", &[]);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "/usr/bin/env\n");
}

#[test]
fn issue1806_env_ignore_environment_prints_and_passes_nothing() {
    if !env_binary() {
        return;
    }
    // `env -i` alone: the composed environment is empty (GNU prints
    // nothing at all — in particular no `_`).
    let (stdout, _, code) = rubash("env -i", &[]);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "");
    // `env -i A=1 prog`: the child sees exactly the given assignment.
    // (Run in the -c tail position itself: the exec-replace seam used
    // to re-assert an adjusted SHLVL over the composed environment
    // after composition — that residual was rubash#1832, fixed by
    // folding the adjustment into the composition, so the tail form
    // now asserts the same contract directly.)
    let (stdout, _, code) = rubash("env -i A=1 /usr/bin/env", &[]);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "A=1\n");
}
