//! rubash#1487 — `shopt -s -o` with no option names listed ALL 27
//! set -o options instead of only the ENABLED ones. GNU: bare
//! `shopt -s -o` lists just the set ones (braceexpand, hashall,
//! interactive-comments in a plain script shell); `shopt -o` lists
//! all 27.
//!
//! Probe: clusters/x3/repro/1487.sh.
//! Expectations verified against GNU bash 5.3.20 (script files).

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
fn shopt_s_o_lists_only_enabled_options() {
    let (stdout, stderr, code) = rubash(
        "shopt -s -o | while read -r n r; do echo \"on:$n\"; done\n\
         echo ---\n\
         shopt -s -o | wc -l\n\
         shopt -o | wc -l",
    );
    assert_eq!(
        stdout,
        "on:braceexpand\non:hashall\non:interactive-comments\n---\n3\n27\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
