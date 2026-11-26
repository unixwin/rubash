//! rubash#202 — `declare -f` must replay the function's definition,
//! not a re-serialization polluted by the definition CONTEXT.
//!
//! GNU execute_cmd.c bind_function_def: the definition's original
//! text is preserved at bind time and declare -f replays it. Two
//! filed symptom classes are pinned here:
//! 1. Functions defined while a SOURCE-level redirect is active
//!    (`. ./inner.sh >/dev/null 2>/dev/null`) must not inherit that
//!    redirect into their printed closing brace — the rubash bug
//!    printed `} 2>> /dev/null` (leaked, with the operator mutated
//!    to append) where GNU prints a plain `}`.
//! 2. In a compound `[[ ]]`, GNU's print makes the implicit unary
//!    explicit: `[[ -n $1 && ${2-x} ]]` prints back as
//!    `[[ -n $1 && -n ${2-x} ]]` — the rubash print dropped the
//!    second `-n` entirely.
//!
//! Expectations byte-verified against GNU bash 5.3.20 (script files
//! for the source form, `-c` for the print form).

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash202-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

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
fn declare_f_ignores_source_context_redirect() {
    let dir = tempdir("srcctx");
    std::fs::write(dir.join("inner.sh"), "pf() { echo PF; }\n").expect("write inner");
    std::fs::write(
        dir.join("outer.sh"),
        ". ./inner.sh >/dev/null 2>/dev/null\ndeclare -f pf\n",
    )
    .expect("write outer");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("outer.sh")
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(stdout, "pf () \n{ \n    echo PF\n}\n");
    assert_eq!(output.status.code(), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn declare_f_makes_implicit_unary_explicit_in_cond() {
    let (stdout, _, code) = rubash("nf() { [[ -n $1 && ${2-x} ]]; }; declare -f nf");
    assert_eq!(stdout, "nf () \n{ \n    [[ -n $1 && -n ${2-x} ]]\n}\n");
    assert_eq!(code, Some(0));
}
