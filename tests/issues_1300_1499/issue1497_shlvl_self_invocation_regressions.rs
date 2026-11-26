//! rubash#1497 — a single-command `-c` self-invocation incremented
//! SHLVL one extra time (no exec-replacement): when a `-c` string is
//! a single simple command, GNU EXECUTES it in place (tail-call), so
//! the inner shell starts from the same exported SHLVL the outer
//! shell itself started from. In a multi-command `-c` string the
//! inner shell is fork+exec'd and sees one more level.
//!
//! Invariant (verified against GNU bash 5.3.20 across env -i, full
//! env, and SHLVL=0/5 starting points): with starting env SHLVL=E,
//! self=E+1, single-command inner=E+1 (== self), multi-command
//! inner=E+2 (== self+1). Probe: clusters/x4/repro/1497.sh (in the
//! probe's chain the middle shell was level 2, hence inner 2 vs 3).
//! The test measures `self` with the same pinned env and asserts the
//! invariant, so it is independent of the harness's own SHLVL.

use std::process::Command;

fn run(bin: &std::path::Path, args: &[&str]) -> (String, Option<i32>) {
    let output = Command::new(bin)
        .args(args)
        .env("SHLVL", "0")
        .output()
        .expect("run shell");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.code(),
    )
}

#[test]
fn single_command_dash_c_self_invocation_shlvl() {
    let bin = std::path::PathBuf::from(env!("CARGO_BIN_EXE_rubash"));
    let bin_str = bin.display().to_string();
    // Baseline: the outer shell's own level under the pinned env.
    let (self_out, _) = run(&bin, &["-c", "echo $SHLVL"]);
    let self_level: i64 = self_out.trim().parse().expect("numeric SHLVL");
    // Single-command form: the outer shell execs the inner one; the
    // inner level equals the outer shell's own level.
    let (stdout, code) = run(&bin, &["-c", "\"$0\" -c 'echo \"inner=$SHLVL\"'", &bin_str]);
    assert_eq!(stdout, format!("inner={self_level}\n"));
    assert_eq!(code, Some(0));
    // Control: multi-command form forks, one more level.
    let (stdout2, code2) = run(
        &bin,
        &[
            "-c",
            "\"$0\" -c 'echo \"inner2=$SHLVL\"'; echo outer-done",
            &bin_str,
        ],
    );
    assert_eq!(stdout2, format!("inner2={}\nouter-done\n", self_level + 1));
    assert_eq!(code2, Some(0));
}
