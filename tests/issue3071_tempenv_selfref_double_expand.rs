//! Issue rubash#3071: a command-prefix (tempenv) assignment whose RHS
//! references the assigned variable itself was expanded TWICE on its way
//! into an external child's environment — once by
//! apply_temporary_assignments (which binds the once-expanded value into
//! the variable table) and again by compose_external_environment, which
//! re-expanded the raw RHS against the already-bound table and
//! superseded the composed pair. `v="a b"; v="p${v}q" printenv v`
//! printed `ppa bqq`.
//!
//! GNU model: subst.c:13153 do_assignment_statements picks
//! variables.c:3536 assign_in_env for a non-null command; assign_in_env
//! expands the RHS exactly once via subst.c:3881
//! expand_assignment_string_to_string (against the values visible before
//! this assignment's own bind, with earlier prefix binds of the same
//! list already in temporary_env) and pre-composes the export string
//! (variables.c:3598 + mk_env_string), which reaches the child verbatim.
//!
//! Expectations are the literal GNU bash 5.3.20 outputs for the same
//! scripts (script-file probes, oracle-verified).
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
fn issue3071_selfref_prefix_expands_once_into_child_env() {
    let (out, _) = run(r#"
v="a b"; v="p${v}q" /usr/bin/printenv v
v="a b"; v="p${v}q${v}r" /usr/bin/printenv v
v="a b"; v="p${v:-d}q" /usr/bin/printenv v
"#);
    assert_eq!(out, "pa bq\npa bqa br\npa bq\n");
}

#[test]
fn issue3071_path_splice_prefix() {
    let (out, _) = run(r#"
PATH="/a:/b"; PATH="/opt:$PATH" /usr/bin/printenv PATH
"#);
    assert_eq!(out, "/opt:/a:/b\n");
}

#[test]
fn issue3071_prefix_list_ordering_and_cross_refs() {
    // Later prefix binds see earlier prefix binds of the same list
    // (GNU temporary_env), including a repeated name; a cross-name
    // reference reads the prefix value, not the outer one.
    let (out, _) = run(r#"
v="z"; v="a b" v="p${v}q" /usr/bin/printenv v
a="OLD"; a="1" b="$a" /usr/bin/printenv b
v="a b"; w="c d"; v="p${v}q" w="r${w}s" /usr/bin/printenv v
v="a b"; w="c d"; v="p${v}q" w="r${w}s" /usr/bin/printenv w
"#);
    assert_eq!(out, "pa bq\n1\npa bq\nrc ds\n");
}

#[test]
fn issue3071_selfref_spawn_shape_variants() {
    // Every spawn route that composes the child environment from the
    // command node must carry the once-expanded value: background,
    // command substitution capture, process substitution producer,
    // explicit subshell, and a child shell reading its environment.
    let (out, _) = run(r#"
v="a b"; v="p${v}q" /usr/bin/printenv v & wait
v="a b"; out=$(v="p${v}q" /usr/bin/printenv v); echo "captured: $out"
v="a b"; cat <(v="p${v}q" /usr/bin/printenv v)
v="a b"; ( v="p${v}q" /usr/bin/printenv v )
v="a b"; v="p${v}q" /bin/sh -c 'echo "sh sees: $v"'
"#);
    assert_eq!(
        out,
        "pa bq\ncaptured: pa bq\npa bq\npa bq\nsh sees: pa bq\n"
    );
}

#[test]
fn issue3071_selfref_with_substitution_and_unset_target() {
    let (out, _) = run(r#"
v="a b"; v="$(echo c)${v}" /usr/bin/printenv v
unset u; u="p${u}q" /usr/bin/printenv u
v="a*b"; v="p${v}q" /usr/bin/printenv v
"#);
    assert_eq!(out, "ca b\npq\npa*bq\n");
}

#[test]
fn issue3071_function_context_shapes() {
    // A function body command carries its own prefix: one expansion.
    // A pipeline element inside a tempenv'd function call is composed
    // by the pipeline spawner (no apply on this executor): the
    // element's RHS expands against the function's tempenv value —
    // the outer frame's records must not be picked up for it, even
    // when the assignment text is identical to the call's own prefix.
    let (out, _) = run(r#"
f() { v="p${v}q" /usr/bin/printenv v; }
v="a b"; v="OUTER" f
g() { v="p${v}q" /usr/bin/printenv v | cat; }
v="a b"; v="p${v}q" g
h() { /usr/bin/printenv v; }
v="a b"; v="p${v}q" h
echo "shell v: $v"
"#);
    assert_eq!(out, "pOUTERq\nppa bqq\npa bq\nshell v: a b\n");
}

#[test]
fn issue3071_controls_unchanged() {
    // Shapes that were already correct must not move: appends (plain
    // and self-referencing), the pipeline spawner path, non-self
    // references, the table value a builtin argument expansion sees
    // (expanded before the tempenv bind), and the post-command value.
    let (out, _) = run(r#"
v="a b"; v+="X" /usr/bin/printenv v
v="a b"; v+="p${v}q" /usr/bin/printenv v
v="a b"; v="p${v}q" /usr/bin/printenv v | cat
v="a b"; w="p${v}q" /usr/bin/printenv w
v="a b"; v="p${v}q" echo "shell sees: $v"
echo "after: $v"
v="a b"; v="p$((1+1))q" /usr/bin/printenv v
"#);
    assert_eq!(
        out,
        "a bX\na bpa bq\npa bq\npa bq\nshell sees: a b\nafter: a b\np2q\n"
    );
}
