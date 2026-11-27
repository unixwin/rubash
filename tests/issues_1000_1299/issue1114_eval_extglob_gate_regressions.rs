//! rubash#1114 — Code parsed by `eval` ignores whether extglob is
//! enabled.
//!
//! GNU: `eval` re-parses its argument with the CURRENT shopt state.
//! With extglob never enabled, an extended-glob pattern inside eval'd
//! code is a syntax error (rc=2); the same pattern written directly
//! after `shopt -s extglob` matches. Only the eval parse path may not
//! lose the gate.
//!
//! Expectations verified against GNU bash 5.3.20 (`-c` string): the
//! syntax-error wording is matched on its stable core (the line prefix
//! carries the binary's own path), the rc markers and the post-enable
//! match are byte-exact.

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn eval_parse_honors_extglob_gate() {
    let (stdout, _, code) = rubash(
        "v=\"abc123\"\n\
         eval 'case $v in +([a-z])+([0-9])) echo m1;; *) echo no;; esac' 2>&1; \
         echo \"pre-extglob-rc=$?\"\n\
         shopt -s extglob\n\
         case $v in +([a-z])+([0-9])) echo m2;; *) echo no2;; esac",
    );
    assert!(
        stdout.contains("syntax error near unexpected token `('"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("pre-extglob-rc=2\n"), "stdout: {stdout}");
    assert!(stdout.ends_with("m2\n"), "stdout: {stdout}");
    // `m1` may appear inside the offending-line echo of the eval parse
    // diagnostic (the echoed line literally contains `echo m1`), so a
    // plain substring check is unsatisfiable against the GNU-shaped
    // output; it must never appear as an executed output line. Same
    // construction as the #1609 companion pin in
    // `issue1609_eval_parse_diag_routing_regressions.rs`.
    assert!(!stdout.lines().any(|line| line == "m1"), "stdout: {stdout}");
    assert_eq!(code, Some(0));
}
