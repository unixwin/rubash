//! Issue rubash#1620: a compound assignment in a command's temporary
//! environment (`a+=(9) cmd`) dropped the target's existing value — the
//! window saw only the literal list text — because the compound branch of
//! apply_temporary_assignments ignored the append flag entirely.
//!
//! GNU model (variables.c assign_in_env + make_variable_value): the
//! tempenv binding is a fresh scalar whose text is the literal list text;
//! with ASS_APPEND the old cell's scalar view is prepended (indexed array
//! -> element 0, associative array -> empty, scalar -> its value).
//!
//! Expectations are the literal GNU bash 5.3.20 outputs for the same
//! scripts.
#![cfg(unix)]

use std::process::Command;

fn run(script: &str) -> (String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn issue1620_tempenv_compound_append_keeps_scalar_base() {
    let (out, _) = run(r#"
f() { echo "win:[$a]"; }
a=1; a+=(9) f
a=1; a+=(9) env | grep '^a='
s=ab; s+=(x) env | grep '^s='
a=1; a+=(9) declare -p a
"#);
    assert_eq!(out, "win:[1(9)]\na=1(9)\ns=ab(x)\ndeclare -x a=\"1(9)\"\n");
}

#[test]
fn issue1620_tempenv_compound_append_array_targets_use_scalar_view() {
    let (out, _) = run(r#"
declare -a arr=(1 2); arr+=(9) bash -c 'echo "arr-ext:[$arr]"'
declare -A A=([j]=2); A+=( [j]=3 ) bash -c 'declare -p A' | sed 's/declare -[-a-z]* //'
declare -A A=([j]=2); A+=( [k]=3 ) bash -c 'declare -p A' | sed 's/declare -[-a-z]* //'
"#);
    assert_eq!(out, "arr-ext:[1(9)]\nA=\"([j]=3)\"\nA=\"([k]=3)\"\n");
}

#[test]
fn issue1620_tempenv_compound_non_append_and_restore_unchanged() {
    let (out, _) = run(r#"
f() { echo "win:[$a]"; }
a=1; a=(9) f
a=1; a=(9) env | grep '^a='
a=; a+=(9) f
a=1; a+=(9) f; echo "after:[$a]"
a=1; a+=5 env | grep '^a='
"#);
    assert_eq!(
        out,
        "win:[(9)]\na=(9)\nwin:[(9)]\nwin:[1(9)]\nafter:[1]\na=15\n"
    );
}

#[test]
fn issue1620_tempenv_compound_append_multiple_and_unset() {
    let (out, _) = run(r#"
a=1; b=2; a+=(9) b+=(x) env | grep -E '^[ab]=' | sort
unset u; u+=(9) env | grep '^u='
"#);
    assert_eq!(out, "a=1(9)\nb=2(x)\nu=(9)\n");
}

#[test]
fn issue1620_tempenv_append_child_env_pipeline_uses_true_external() {
    // `env` is a rubash builtin and prints the shell's own view, so the
    // child-environment assertions must go through the real external
    // binary by path, in pipeline position (the buffered stage spawner
    // re-applied the raw RHS: base lost, marker leaked).
    let (out, _) = run(r#"
a=1; a+=(9) /usr/bin/env | grep '^a='
a=14; a+=5 /usr/bin/env | grep '^a='
declare -a r=(1 2); r+=(9) /usr/bin/env | grep '^r='
declare -A A=([j]=2); A+=( [j]=3 ) /usr/bin/env | grep '^A='
a=1; a+=(9) /usr/bin/env
"#);
    assert!(out.contains("a=1(9)\n"), "{out}");
    assert!(out.contains("a=145\n"), "{out}");
    assert!(out.contains("r=1(9)\n"), "{out}");
    assert!(out.contains("A=([j]=3)\n"), "{out}");
    assert!(!out.contains("__RUBASH_CA1__"), "{out}");
    // The live carrier spelling since #1634 is U+E31A; it must not leak
    // into a child's environment either.
    assert!(!out.contains('\u{E31A}'), "{out}");
}
