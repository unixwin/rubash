//! Issue rubash#362 — a command substitution with a COMPOUND body
//! (`v=$(IFS=:; echo ...)`, `v=$( { ...; } )`, `v=$(a; b)`) captured EMPTY
//! when the enclosing function call carried a stdout redirection
//! (`f >/dev/null`): the body's writes followed the caller's redirected
//! fd 1 into the outer target while the capture read nothing. direnv
//! stdlib path_add() — `path=$(IFS=:; echo "${path_array[*]}")` under
//! `layout go >/dev/null` — exported an EMPTY PATH.
//!
//! GNU anchor: subst.c:7143 command_substitute — the forked child does
//! dup2(fildes[1], 1) (subst.c:7306-7320), so fd 1 is the capture pipe
//! for the WHOLE body, compound or not; an enclosing function-call
//! redirect is irrelevant because the substitution child's descriptor
//! changes never touch the parent (execute_cmd.c redirection stacking).
//!
//! Root cause: rubash has three in-process comsub body runners. The
//! subshell path (command_substitution.rs
//! command_list_substitution_output_typed) and the function shortcut
//! (embedded_mutations.rs run_function_command_substitution, rubash#161)
//! both rebind fd 1 to the Stdout endpoint (which resolves to the active
//! capture) for the body's duration. The compound-body AST path
//! (embedded_mutations.rs run_ast_command_substitution_with_context) ran
//! the body IN PLACE without that rebind, so under an active caller
//! redirect the body's builtin writes followed fd 1 into the outer
//! target. Fixed by bracketing the same fd-1 rebind there;
//! saved_fd_table restores the caller's binding afterwards.
//!
//! Expected outputs byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-28; 16-shape matrix
//! under target/p0fix/i362 in the lane worktree — 15/16 byte-identical,
//! the exception is the pre-existing `exec 3>&1` dup-snapshot semantics,
//! identical on master).
//!
//! Pre-existing residual (NOT this fix): `exec 3>&1; v=$(echo x >&3)`
//! captures `x` in rubash (fd 3 aliases fd 1 dynamically) while GNU's
//! fd 3 snapshots the pre-pipe stdout — dup2-snapshot family, unchanged
//! by this fix.

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

/// The issue's headline: `;`-compound body under `f >/dev/null`.
#[test]
fn compound_comsub_under_function_redirect() {
    let (stdout, stderr, code) =
        rubash("f2() { v=$(IFS=:; echo hi); echo \"compound=[$v]\" >&2; }\nf2 >/dev/null");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "compound=[hi]\n");
    assert_eq!(code, Some(0));
}

/// Group body `{ ...; }`.
#[test]
fn group_comsub_under_function_redirect() {
    let (stdout, stderr, code) =
        rubash("f3() { v=$( { echo hi; } ); echo \"group=[$v]\" >&2; }\nf3 >/dev/null");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "group=[hi]\n");
    assert_eq!(code, Some(0));
}

/// Sequence body `a; b`.
#[test]
fn sequence_comsub_under_function_redirect() {
    let (stdout, stderr, code) =
        rubash("f4() { v=$(printf a; printf b); echo \"seq=[$v]\" >&2; }\nf4 >/dev/null");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "seq=[ab]\n");
    assert_eq!(code, Some(0));
}

/// Compound + parameter operator on the captured value (direnv
/// path_add's `${path%%:*}`-style use).
#[test]
fn compound_comsub_paramop_under_redirect() {
    let (stdout, stderr, code) = rubash(
        "fp() { v=$(IFS=:; echo \"a:b:c\"); echo \"paramop=[${v%%:*}]\" >&2; }\nfp >/dev/null",
    );
    assert_eq!(stdout, "");
    assert_eq!(stderr, "paramop=[a]\n");
    assert_eq!(code, Some(0));
}

/// The direnv path_add shape: array join inside a redirected call.
#[test]
fn array_join_comsub_under_redirect() {
    let (stdout, stderr, code) = rubash(
        "fa() { arr=(\"/x\" \"/y\"); v=$(IFS=:; echo \"${arr[*]}\"); echo \"arrjoin=[$v]\" >&2; }\nfa >/dev/null",
    );
    assert_eq!(stdout, "");
    assert_eq!(stderr, "arrjoin=[/x:/y]\n");
    assert_eq!(code, Some(0));
}

/// The outer redirect target must NOT receive the body's output.
#[test]
fn body_output_does_not_leak_into_outer_target() {
    let (stdout, stderr, code) = rubash(
        "f2() { v=$(IFS=:; echo hi); echo \"compound=[$v]\" >&2; }\nf2 > /tmp/i362-outer.txt\ncat /tmp/i362-outer.txt",
    );
    assert_eq!(stdout, "");
    assert_eq!(stderr, "compound=[hi]\n");
    assert_eq!(code, Some(0));
}

/// Group/for enclosing redirects (same class as the function call).
#[test]
fn group_and_loop_redirects_capture() {
    let (stdout, stderr, code) = rubash(
        "g1() { v=$(echo ghi); echo \"g=[$v]\" >&2; }\n{ g1; } >/dev/null\n\
         for i in 1 2; do v=$(echo loop$i); echo \"loop=[$v]\" >&2; done >/dev/null",
    );
    assert_eq!(stdout, "");
    assert_eq!(stderr, "g=[ghi]\nloop=[loop1]\nloop=[loop2]\n");
    assert_eq!(code, Some(0));
}

/// Simple bodies keep working under the same redirect (regression guard).
#[test]
fn simple_comsub_under_function_redirect() {
    let (stdout, stderr, code) =
        rubash("f0() { v=$(echo hi); echo \"simple=[$v]\" >&2; }\nf0 >/dev/null");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "simple=[hi]\n");
    assert_eq!(code, Some(0));
}

/// The body's OWN stdout redirect still overrides the capture for that
/// command only; later commands return to the capture.
#[test]
fn body_level_redirect_scoping() {
    let (stdout, stderr, code) =
        rubash("v=$(echo drop > /dev/null; echo second); echo \"bodyredir=[$v]\" >&2");
    assert_eq!(stdout, "");
    assert!(stderr.contains("bodyredir=[second]"), "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

/// `2>&1` inside the body joins the capture (fd 1 target), matching GNU.
#[test]
fn body_dup_stderr_into_capture() {
    let (stdout, stderr, code) = rubash("v=$(echo hi 2>&1); echo \"duperr=[$v]\" >&2");
    assert_eq!(stdout, "");
    assert!(stderr.contains("duperr=[hi]"), "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

/// Nested compound comsub under an outer redirect.
#[test]
fn nested_compound_comsub_under_redirect() {
    let (stdout, stderr, code) = rubash(
        "fnest() { w=$(v2=$(echo deep); echo \"[$v2]\"); echo \"nest=$w\" >&2; }\nfnest >/dev/null",
    );
    assert_eq!(stdout, "");
    assert_eq!(stderr, "nest=[deep]\n");
    assert_eq!(code, Some(0));
}
