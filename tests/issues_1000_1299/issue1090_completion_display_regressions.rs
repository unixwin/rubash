//! rubash#1090 — Completion display-text gaps (compopt -D/-E names,
//! -r error, -F warning order).
//!
//! GNU builtins/complete.def: `compopt -D` with no explicit options
//! prints the effective option set for the default completion using the
//! canonical spellings; `compgen -F` emits its "-F option may not work
//! as you expect" warning before the function's own output in a merged
//! stream.
//!
//! Expectations verified against GNU bash 5.3.20 (`-c` string); the
//! compopt line is byte-exact, the warning is matched on its text (the
//! line prefix carries the binary's own path).

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
fn completion_display_text_matches_gnu() {
    let (stdout, _, code) = rubash(
        "complete -W \"dflt\" -D; complete -o filenames -D; compopt -D\n\
         complete -W d -D; complete -r -D\n\
         _g(){ echo FUNC-OUTPUT; }; compgen -F _g foo 2>&1",
    );
    assert!(
        stdout.contains(
            "compopt +o bashdefault +o default +o dirnames -o filenames \
             +o fullquote +o noquote +o nosort +o nospace +o plusdirs -D"
        ),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("compgen: warning: -F option may not work as you expect"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("FUNC-OUTPUT"), "stdout: {stdout}");
    assert_eq!(code, Some(1));
}
