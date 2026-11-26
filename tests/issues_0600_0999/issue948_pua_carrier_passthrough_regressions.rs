//! rubash#948 — Private-use-area characters in a child shell's
//! arguments must pass through byte-exact (catalog case issue-948).
//!
//! GNU subst.c / execute_cmd.c: a PUA codepoint (U+E319) in a word is
//! ordinary data — it is NOT one of the engine's internal carrier
//! bytes — so it survives being passed to a child shell, parsed
//! there, and echoed, in one- and two-argument forms alike. The
//! catalog case drives exactly this through `$THIS_SH -c` children;
//! this pin drives the same three commands at the built binary.
//!
//! Expectations byte-verified against GNU bash 5.3.20.

use std::process::Command;

fn rubash_c(script: &str) -> (String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.code(),
    )
}

#[test]
fn pua_literal_passes_through_child_echo() {
    let (sentinel, code) = rubash_c("echo \"__RUBASH_CSB1_41;\"");
    assert_eq!(sentinel, "__RUBASH_CSB1_41;\n");
    assert_eq!(code, Some(0));

    let (one, code) = rubash_c("echo \"x\u{E319}41;y\"");
    assert_eq!(one, "x\u{E319}41;y\n");
    assert_eq!(code, Some(0));

    let (two, code) = rubash_c("echo \"x\u{E319}41;y\" \"done\"");
    assert_eq!(two, "x\u{E319}41;y done\n");
    assert_eq!(code, Some(0));
}
