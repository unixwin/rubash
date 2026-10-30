//! rubash#1766: the host external-command channel re-applied a command's
//! raw prefix-assignment values into the env map handed to the host handler
//! — including append shapes (raw RHS overwriting the composed value) and
//! carrier-marker (`__RUBASH_CA1__`) compound forms. This is the #1620
//! root-cause-2 shape on the embedded-host channel: the top-level path
//! must skip appends (the dispatch seam already composed them into the
//! table, like apply_external_environment does) and strip the marker, while
//! the pipeline-stage path must compose appends itself (stage assignments
//! are never applied to the table, like the stage spawner does).
//!
//! Every expectation below was measured against GNU bash 5.3.20 with the
//! same shapes through /usr/bin/env:
//!   a=14; a+=5 env -> a=145            a=1; a+=(9) env -> a=1(9)
//!   a=(x y) env -> a=(x y)             a=(10 20); a+=(30) env -> a=10(30)
//!   declare -A m=([k]=v); m+=([j]=w) env -> m=([j]=w)
//!   declare -n ref=var; var=orig; ref+=x env -> var=origx (no ref= line)
//!   unset u; u+=5 env -> u=5
//!
//! The top-level compound-append row (a=1; a+=(9) -> 1(9)) was deliberately
//! weaker than GNU while its value rode the carrier cluster's table fix
//! (PR #1767, #1620 root cause 1); #1767 has merged, so the row now pins
//! the exact `1(9)` value as well as no marker leak.

use rubash::executor::{Executor, HostExternalCommandOutput};
use rubash::lexer::tokenize;
use rubash::parser::parse;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Run `script` with a capturing host handler. The handler answers only
/// for the probe word `hostprobe` (other words fall through to real disk
/// execution, so a pipeline's `cat` stage still runs); one captured env
/// map is returned per `hostprobe` invocation, in order.
fn run_host_captures(script: &str) -> Vec<HashMap<String, String>> {
    let captured: Arc<Mutex<Vec<HashMap<String, String>>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = captured.clone();
    let mut executor = Executor::new();
    executor.set_host_external_command_handler(move |words, env| {
        if words.first().map(String::as_str) != Some("hostprobe") {
            return None;
        }
        sink.lock().unwrap().push(env.clone());
        Some(HostExternalCommandOutput {
            stdout: Vec::new(),
            stderr: Vec::new(),
            status: 0,
        })
    });
    let tokens = tokenize(script);
    let ast = parse(&tokens);
    let result = executor.execute_ast(&ast);
    assert!(result.is_ok(), "script failed: {script:?} -> {result:?}");
    // The executor owns the handler closure, which owns the second Arc
    // clone of the sink; try_unwrap can only succeed once the executor
    // (and with it the closure) is dropped. Without this drop every test
    // in this file panics in try_unwrap before any assertion runs.
    drop(executor);
    Arc::try_unwrap(captured)
        .ok()
        .unwrap()
        .into_inner()
        .unwrap()
}

/// Exactly one `hostprobe` invocation; return its env map.
fn only(script: &str) -> HashMap<String, String> {
    let maps = run_host_captures(script);
    assert_eq!(
        maps.len(),
        1,
        "expected exactly one hostprobe invocation for {script:?}, got {}",
        maps.len()
    );
    maps.into_iter().next().unwrap()
}

fn assert_no_marker(env: &HashMap<String, String>) {
    for (key, value) in env {
        assert!(
            !value.contains("__RUBASH_CA1__") && !value.contains('\u{E31A}'),
            "carrier marker leaked into host env: {key}={value}"
        );
    }
}

// ---- top-level path (dispatch seam applied the assignments) ----

#[test]
fn issue1766_top_scalar_append_composes_window_value() {
    let env = only("a=14; a+=5 hostprobe");
    assert_eq!(env.get("a").map(String::as_str), Some("145"));
    assert_no_marker(&env);
}

#[test]
fn issue1766_top_scalar_plain_overwrites_window_value() {
    let env = only("a=14; a=9 hostprobe");
    assert_eq!(env.get("a").map(String::as_str), Some("9"));
}

#[test]
fn issue1766_top_compound_plain_strips_marker() {
    let env = only("a=(x y) hostprobe");
    assert_eq!(env.get("a").map(String::as_str), Some("(x y)"));
    assert_no_marker(&env);
}

#[test]
fn issue1766_top_compound_append_composes_window_value() {
    // Formerly a weak row (no-marker-leak only) while the value rode the
    // carrier cluster's table fix (PR #1767); #1767 has merged, so this row
    // now pins the exact GNU value 1(9) as well as no marker leak.
    let env = only("a=1; a+=(9) hostprobe");
    assert_eq!(env.get("a").map(String::as_str), Some("1(9)"));
    assert_no_marker(&env);
}

#[test]
fn issue1766_top_unset_append_binds_rhs() {
    let env = only("unset u; u+=5 hostprobe");
    assert_eq!(env.get("u").map(String::as_str), Some("5"));
}

#[test]
fn issue1766_top_multiple_scalar_appends_compose() {
    let env = only("a=1; b=2; a+=9 b+=x hostprobe");
    assert_eq!(env.get("a").map(String::as_str), Some("19"));
    assert_eq!(env.get("b").map(String::as_str), Some("2x"));
    assert_no_marker(&env);
}

#[test]
fn issue1766_top_tempenv_restores_after_host_command() {
    let maps = run_host_captures("a=14; a+=5 hostprobe; hostprobe");
    assert_eq!(maps.len(), 2);
    assert_eq!(maps[0].get("a").map(String::as_str), Some("145"));
    assert_eq!(maps[1].get("a").map(String::as_str), Some("14"));
}

#[test]
fn issue1766_top_nameref_append_binds_target() {
    let env = only("declare -n ref=var; var=orig; ref+=x hostprobe");
    assert_eq!(env.get("var").map(String::as_str), Some("origx"));
    assert!(!env.contains_key("ref"));
    assert_no_marker(&env);
}

// ---- pipeline-stage path (stage assignments never hit the table) ----

#[test]
fn issue1766_stage_scalar_append_composes_window_value() {
    let env = only("a=14; a+=5 hostprobe | cat");
    assert_eq!(env.get("a").map(String::as_str), Some("145"));
    assert_no_marker(&env);
}

#[test]
fn issue1766_stage_scalar_plain_overwrites_window_value() {
    let env = only("a=14; a=9 hostprobe | cat");
    assert_eq!(env.get("a").map(String::as_str), Some("9"));
}

#[test]
fn issue1766_stage_compound_plain_strips_marker() {
    let env = only("a=(x y) hostprobe | cat");
    assert_eq!(env.get("a").map(String::as_str), Some("(x y)"));
    assert_no_marker(&env);
}

#[test]
fn issue1766_stage_compound_append_scalar_composes() {
    let env = only("a=1; a+=(9) hostprobe | cat");
    assert_eq!(env.get("a").map(String::as_str), Some("1(9)"));
    assert_no_marker(&env);
}

#[test]
fn issue1766_stage_compound_append_indexed_uses_element_zero() {
    let env = only("a=(10 20); a+=(30) hostprobe | cat");
    assert_eq!(env.get("a").map(String::as_str), Some("10(30)"));
    assert_no_marker(&env);
}

#[test]
fn issue1766_stage_compound_append_assoc_uses_appended_list() {
    let env = only("declare -A m=([k]=v); m+=([j]=w) hostprobe | cat");
    assert_eq!(env.get("m").map(String::as_str), Some("([j]=w)"));
    assert_no_marker(&env);
}

#[test]
fn issue1766_stage_unset_append_binds_rhs() {
    let env = only("unset u; u+=5 hostprobe | cat");
    assert_eq!(env.get("u").map(String::as_str), Some("5"));
}

#[test]
fn issue1766_stage_nameref_append_binds_target() {
    let env = only("declare -n ref=var; var=orig; ref+=x hostprobe | cat");
    assert_eq!(env.get("var").map(String::as_str), Some("origx"));
    assert!(!env.contains_key("ref"));
    assert_no_marker(&env);
}
