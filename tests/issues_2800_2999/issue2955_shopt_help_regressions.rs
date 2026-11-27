//! rubash#2955 (Stage-2 ④ Phase 3) — `shopt --help` in option
//! position did not print the long help: shopt's hand-written scan
//! had no help face and reported the first `-` of `--help` as an
//! invalid option. GNU builtins/shopt.def runs
//! `internal_getopt (list, "psuoq")`; bashgetopt.c:80-85 returns
//! GETOPT_HELP for a `--help` word in option position and
//! CASE_HELPOPT (builtins/common.h:31-36) prints the long help and
//! returns EX_USAGE. The expected text below is a byte-exact copy
//! of GNU bash 5.3.20's `shopt --help` output (Phase 3 E1).
//!
//! Also pinned: `--help` in OPERAND position (`shopt dotglob
//! --help`) is not a help request — it is an invalid shell option
//! name, exactly as before the migration.

use std::path::PathBuf;
use std::process::Command;

struct Run {
    status: i32,
    stdout: String,
    stderr: String,
    dir: PathBuf,
}

fn run_script(body: &str) -> Run {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("issue2955_{}_{}", std::process::id(), seq));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    std::fs::write(dir.join("case.sh"), body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    Run {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        dir,
    }
}

const EXPECTED_SHOPT_HELP: &str = "shopt: shopt [-pqsu] [-o] [optname ...]\n    Set and unset shell options.\n    \n    Change the setting of each shell option OPTNAME.  Without any option\n    arguments, list each supplied OPTNAME, or all shell options if no\n    OPTNAMEs are given, with an indication of whether or not each is set.\n    \n    Options:\n      -o\trestrict OPTNAMEs to those defined for use with `set -o'\n      -p\tprint each shell option with an indication of its status\n      -q\tsuppress output\n      -s\tenable (set) each OPTNAME\n      -u\tdisable (unset) each OPTNAME\n    \n    Exit Status:\n    Returns success if OPTNAME is enabled; fails if an invalid option is\n    given or OPTNAME is disabled.\n";

#[test]
fn shopt_help_prints_long_help() {
    let run = run_script("shopt --help\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    assert_eq!(run.stdout, EXPECTED_SHOPT_HELP, "stdout: {}", run.stdout);
    let _ = run.dir;
}

#[test]
fn shopt_dash_s_help_prints_long_help_and_applies_nothing() {
    let run = run_script("shopt -s --help\necho \"rc=$?\"\nshopt -q dotglob; echo \"q=$?\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout,
        format!("{EXPECTED_SHOPT_HELP}rc=2\nq=1\n"),
        "stdout: {}",
        run.stdout
    );
    let _ = run.dir;
}

#[test]
fn shopt_help_in_operand_position_is_an_invalid_name() {
    let run = run_script("shopt dotglob --help\necho \"rc=$?\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(
        run.stderr, "case.sh: line 1: shopt: --help: invalid shell option name\n",
        "stderr: {}",
        run.stderr
    );
    assert_eq!(
        run.stdout, "dotglob             \toff\nrc=1\n",
        "stdout: {}",
        run.stdout
    );
    let _ = run.dir;
}
