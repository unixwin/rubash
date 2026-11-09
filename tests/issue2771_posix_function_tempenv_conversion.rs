//! rubash#2771 — POSIX-mode in-function kept-tempenv conversion is
//! unwired (split from rubash#2720). The shadowing predicate
//! correctly makes a kept temporary-environment binding win over
//! the dynamic provider in POSIX mode too, but the follow-on
//! CONVERSION GNU performs for a kept tempenv binding inside a
//! function under POSIX mode is not wired — the value lands as a
//! plain static cell instead of becoming the real dynamic bind.
//!
//! GNU ground (bash 5.3.20 oracle, script-file probes): in POSIX
//! mode, a tempenv assignment preceding a special builtin persists
//! after the builtin (POSIX 2.9.1) — inside a function too — and
//! the persistence merge rebinds through the underlying variable
//! (variables.c merge_temporary_env -> bind_variable), so for a
//! hook-carrying dynamic name the assignment hook runs on the kept
//! value: `SECONDS=100 :` leaves $SECONDS ticking from 100 (the
//! offset hook), `RANDOM=7 :` leaves the generator seeded (reads
//! draw, they do not read back "7"). The converted bind lands on
//! the global dynamic cell: it is still in force after the
//! function returns. Rubash wires this conversion for the
//! attribute-absorption and top-level arms
//! (Executor::convert_kept_tempenv_dynamic_binds) but the POSIX
//! in-function merge arm in src/executor/command_dispatch.rs never
//! calls it. Expectations are the literal oracle outputs.
#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2771-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn run_script(dir: &Path, script: &str) -> (String, String) {
    fs::write(dir.join("probe.sh"), script).expect("write probe script");
    let output = Command::new(RUBASH)
        .current_dir(dir)
        .arg("probe.sh")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", dir)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn check(tag: &str, script: &str, expected_stdout: &str) {
    let dir = scratch(tag);
    let (stdout, stderr) = run_script(&dir, script);
    assert_eq!(stdout, expected_stdout, "{tag}: stdout (stderr: {stderr})");
    assert_eq!(stderr, "", "{tag}: stderr");
}

/// The defect in one chain: the kept SECONDS bind inside the
/// function reads back as the assigned base, a nested call sees the
/// same live value, and one second later it has TICKED — the
/// offset hook ran on the kept value. An unconverted plain cell
/// would read 100 at every step.
#[test]
fn issue2771_posix_in_function_seconds_converts_and_ticks() {
    check(
        "seconds-chain",
        "set -o posix\ng() { echo \"in-g: $SECONDS\"; }\nf() { SECONDS=100 :; echo \"in-f: $SECONDS\"; g; sleep 1; echo \"in-f2: $SECONDS\"; }\nf\n",
        "in-f: 100\nin-g: 100\nin-f2: 101\n",
    );
}

/// The converted bind is the global dynamic cell, not a frame
/// cell: after the function returns, the top level still reads the
/// converted clock (100 at bind time, 101 after a further second).
#[test]
fn issue2771_posix_in_function_conversion_reaches_global_cell() {
    check(
        "seconds-global",
        "set -o posix\nf() { SECONDS=100 :; }\nf\nsleep 1\necho \"top-after: $SECONDS\"\n",
        "top-after: 101\n",
    );
}

/// RANDOM: after the kept bind converts, reads DRAW from the seed
/// — two consecutive reads differ and neither is the raw assigned
/// value. (The numeric sequence is the shell's own PRNG and is not
/// compared against GNU's; the converted-vs-plain distinction is
/// the property under test. An unconverted cell reads "7,7".)
#[test]
fn issue2771_posix_in_function_random_converts_to_draws() {
    let dir = scratch("random-draws");
    let (out, stderr) = run_script(
        &dir,
        "set -o posix\nf() { RANDOM=7 :; echo \"draws: $RANDOM,$RANDOM\"; }\nf\n",
    );
    assert_eq!(stderr, "", "stderr");
    let draws: Vec<&str> = out
        .trim()
        .strip_prefix("draws: ")
        .expect("draws line")
        .split(',')
        .collect();
    assert_eq!(draws.len(), 2, "two draws: {out}");
    assert_ne!(draws[0], "7", "first read must draw, not read the plain cell: {out}");
    assert_ne!(draws[1], "7", "second read must draw, not read the plain cell: {out}");
    assert_ne!(
        draws[0], draws[1],
        "consecutive reads of a seeded generator must differ: {out}"
    );
}

/// Control (expected to pass before and after the fix): the same
/// kept conversion at TOP level in POSIX mode is already wired —
/// this is the arm the in-function path must be brought in line
/// with, not a second defect.
#[test]
fn issue2771_posix_top_level_conversion_already_wired() {
    check(
        "top-level-control",
        "set -o posix\nSECONDS=100 :\ns=$SECONDS; sleep 1; echo \"top: $s -> $SECONDS\"\n",
        "top: 100 -> 101\n",
    );
}

/// Control: in DEFAULT mode nothing is kept, in a function or
/// anywhere else — the tempenv window closes with the builtin and
/// $SECONDS is the shell's own clock (0 at start, 1 after the
/// sleep). The fix must not let the conversion leak into the
/// default-mode path.
#[test]
fn issue2771_default_mode_keeps_nothing() {
    check(
        "default-control",
        "f() { SECONDS=100 :; s=$SECONDS; sleep 1; echo \"in-f: $s -> $SECONDS\"; }\nf\n",
        "in-f: 0 -> 1\n",
    );
}
