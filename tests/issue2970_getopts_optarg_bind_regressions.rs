//! rubash#2970 — `getopts` on a valid option that takes NO argument
//! unbound `OPTARG` wholesale (value AND the variable itself, readonly
//! mark included); GNU binds it.
//!
//! GNU mechanism (builtins/getopts.def): every exit path of
//! dogetopts is explicit about OPTARG —
//! - valid option (getopts.def:301): `bind_variable ("OPTARG",
//!   sh_optarg, 0)` runs unconditionally; for a no-argument option
//!   sh_optarg is NULL, so the variable is BOUND WITH NO VALUE
//!   (`declare -p OPTARG` prints `declare -- OPTARG`), and a readonly
//!   OPTARG reports `OPTARG: readonly variable` and keeps its value.
//! - G_EOF (getopts.def:262-267), G_INVALID_OPT and G_ARG_MISSING in
//!   the non-silent forms: `getopts_unbind_variable ("OPTARG")`.
//! - silent (`:`-prefixed) invalid/missing forms bind OPTARG to the
//!   option character instead.
//!
//! Pre-fix rubash called getopts_unbind_optarg() on the valid
//! no-argument path too, so the readonly diagnostic never fired and a
//! previously bound OPTARG vanished.
//!
//! Expectations measured against GNU bash 5.3.20 (oracle binary),
//! script probes 2026-11-07.

use std::process::{Command, Stdio};
use std::time::Duration;

mod common;
use common::wait_bounded;

fn run_case(script: &str) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(["-c", script])
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    let out = wait_bounded(&mut child, Duration::from_secs(20));
    assert!(!out.timed_out, "case timed out: {script}");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.code,
    )
}

#[test]
fn issue2970_noarg_option_reports_readonly_optarg_and_keeps_value() {
    let (stdout, stderr, code) = run_case(
        "readonly OPTARG=old; getopts \"ab:\" opt -a; echo \"rc=$? opt=$opt OPTARG=$OPTARG\"",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0 opt=a OPTARG=old\n");
    assert!(
        stderr.contains("OPTARG: readonly variable"),
        "readonly diagnostic missing, stderr: {stderr}"
    );
}

#[test]
fn issue2970_noarg_option_binds_optarg_without_value() {
    let (stdout, _, _) = run_case("getopts \"ab:\" opt -a; declare -p OPTARG");
    assert_eq!(stdout, "declare -- OPTARG\n");
}

#[test]
fn issue2970_noarg_option_clears_an_earlier_argument_value() {
    // bind_variable with a NULL value replaces the old value with
    // "bound, no value" — the variable does not revert to unbound.
    let (stdout, _, _) = run_case(
        "OPTIND=1; getopts \"ab:\" opt -b X; OPTIND=1; getopts \"ab:\" opt -a; declare -p OPTARG",
    );
    assert_eq!(stdout, "declare -- OPTARG\n");
}

#[test]
fn issue2970_error_paths_still_unbind_optarg() {
    // G_INVALID_OPT, non-silent: OPTARG ends unbound.
    let (stdout, _, _) =
        run_case("getopts \"ab:\" opt -z 2>/dev/null; declare -p OPTARG 2>&1; echo \"rc=$?\"");
    assert!(stdout.contains("OPTARG: not found"), "stdout: {stdout}");
    assert!(stdout.contains("rc=1"), "stdout: {stdout}");
    // G_ARG_MISSING, non-silent: OPTARG ends unbound.
    let (stdout, _, _) =
        run_case("getopts \"ab:\" opt -b 2>/dev/null; declare -p OPTARG 2>&1; echo \"rc=$?\"");
    assert!(stdout.contains("OPTARG: not found"), "stdout: {stdout}");
    // G_EOF: OPTARG ends unbound.
    let (stdout, _, _) = run_case("getopts \"ab:\" opt; declare -p OPTARG 2>&1; echo \"rc=$?\"");
    assert!(stdout.contains("OPTARG: not found"), "stdout: {stdout}");
}

#[test]
fn issue2970_silent_error_paths_still_bind_the_option_character() {
    let (stdout, _, _) = run_case("getopts \":ab:\" opt -z; echo \"opt=$opt OPTARG=$OPTARG\"");
    assert_eq!(stdout, "opt=? OPTARG=z\n");
    let (stdout, _, _) = run_case("getopts \":ab:\" opt -b; echo \"opt=$opt OPTARG=$OPTARG\"");
    assert_eq!(stdout, "opt=: OPTARG=b\n");
}

#[test]
fn issue2970_argument_option_still_binds_its_argument() {
    let (stdout, _, _) = run_case("getopts \"ab:\" opt -b val; declare -p OPTARG");
    assert_eq!(stdout, "declare -- OPTARG=\"val\"\n");
}
