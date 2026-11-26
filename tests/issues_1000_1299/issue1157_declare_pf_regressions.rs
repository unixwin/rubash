//! rubash#1157 — declare -pF listing drops `declare -f` prefix.
//!
//! GNU builtins/declare.def: in listing mode `-pF` prints
//! `declare -f <name>` lines (the -p prefix form), while `-F` alone
//! prints bare names and `-pf` prints the function definitions.
//! The three spellings are distinct surfaces and must not collapse.
//!
//! Expectations byte-verified against GNU bash 5.3.20 (`-c` string).

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
fn declare_pf_listing_keeps_prefix() {
    let (stdout, _, code) = rubash(
        "myfn(){ :; }\n\
         declare -pF myfn\n\
         declare -p -f -F myfn\n\
         declare -F myfn\n\
         declare -pf myfn\n",
    );
    assert_eq!(
        stdout,
        "declare -f myfn\ndeclare -f myfn\nmyfn\nmyfn () \n{ \n    :\n}\n"
    );
    assert_eq!(code, Some(0));
}
