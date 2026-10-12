//! Issue rubash#539: substring offset/length operands reach evalexp as RAW
//! text (GNU subst.c verify_substring_values → evalexp) — a single-quoted
//! span is a string operand and fails with an arithmetic error (`x: '2':
//! arithmetic syntax error: operand expected`), while the double-quoted
//! form IS stripped (`${x:1:"2"}` -> `bc`). The extquote `$'…'`-as-name
//! decode (huck#8) applies in double-quote context, including substring
//! offsets and command-substitution bodies: `"${x:${$'1'}}"` -> abcdef and
//! `"$(echo ${$'a'})"` -> zz, while an unquoted top-level `${$'a'}` stays a
//! bad substitution.

#![cfg(windows)]

use std::process::{Command, Stdio};

fn run_c(script: &str) -> (Option<i32>, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash -c");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn single_quoted_operands_are_arithmetic_errors() {
    // WSL GNU bash 5.3.0:
    //   bash: line 1: x: '2': arithmetic syntax error: operand expected
    //   (error token is "'2'")   rc=1
    for script in [
        "x=abcdef; echo ${x:1:'2'}",
        "x=abcdef; echo \"${x:1:'2'}\"",
        "x=abcdef; echo ${x:'1':2}",
    ] {
        let (code, stdout, stderr) = run_c(script);
        assert_eq!(stdout, "", "{script:?} must print nothing");
        assert_eq!(code, Some(1), "stderr={stderr:?}");
        let first = stderr.lines().next().unwrap_or_default();
        assert!(
            first.ends_with(
                "x: '2': arithmetic syntax error: operand expected (error token is \"'2'\")"
            ) || first.ends_with(
                "x: '1': arithmetic syntax error: operand expected (error token is \"'1'\")"
            ),
            "stderr for {script:?}: {first:?}"
        );
    }
}

#[test]
fn double_quoted_operands_and_plain_forms_stay_correct() {
    // Both sides strip double quotes in substring operands.
    let (_, stdout, _) = run_c("x=abcdef; echo ${x:1:\"2\"}");
    assert_eq!(stdout, "bc\n");
    let (_, stdout, _) = run_c("x=abcdef; echo ${x:1:2}");
    assert_eq!(stdout, "bc\n");
}

#[test]
fn extquote_dollar_quoted_name_decodes_in_quoted_context() {
    // huck#8 (a): bash `x=abcdef; echo "${x:${$'1'}}"` -> abcdef.
    let (_, stdout, stderr) = run_c("x=abcdef; echo \"${x:${$'1'}}\"");
    assert_eq!(stdout, "abcdef\n", "stderr={stderr:?}");
    assert_eq!(_code(&stderr), "");

    // huck#8 (c): the command-substitution body decodes too.
    let (_, stdout, _) = run_c("a=zz; echo \"$(echo ${$'a'})\"");
    assert_eq!(stdout, "zz\n");

    // The unquoted top-level form stays a bad substitution (rc 1).
    let (code, _, stderr) = run_c("a=zz; echo ${$'a'}");
    assert_eq!(code, Some(1));
    assert!(stderr.contains("bad substitution"), "stderr={stderr:?}");
}

fn _code(_stderr: &str) -> String {
    String::new()
}
