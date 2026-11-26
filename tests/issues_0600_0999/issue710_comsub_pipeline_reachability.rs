// rubash#710: the word-level command-substitution pipeline-filter arms
// (`command_substitution_pipeline_filter` and its caller chain) were
// unreachable — `command_substitution_body_is_trivial` rejects `|`, so every
// `$( … | … )` body already routed through the real parser/executor — and the
// whole dormant chain was removed.
//
// These tests pin the ROUTING, per the issue's ask: if a future change ever
// regresses a pipeline substitution back to a word-level shortcut that
// silently ignores the pipeline (the #70/#76 failure family), the observable
// signatures below diverge. The decisive one is `wc` with no operand: the
// removed dormant arm defaulted to `-l` (one column, `1`), while the real
// executor's pipeline produces GNU's three-column `lines words bytes`
// (`      1       1       6` for `printf 'hello\n'`).

use std::process::Command;

fn run(cmd: &str) -> (Vec<u8>, i32) {
    let exe = env!("CARGO_BIN_EXE_rubash");
    let out = Command::new(exe)
        .arg("-c")
        .arg(cmd)
        .output()
        .expect("spawn rubash");
    (out.stdout, out.status.code().unwrap_or(-1))
}

/// Decisive signature: `$(producer | wc)` with no operand must be the real
/// executor's three-column output, not the removed dormant arm's one-column
/// line count.
#[test]
fn wc_default_is_three_column_real_executor_output() {
    let (out, rc) = run("x=$(printf 'hello\n' | wc); printf '%s\n' \"$x\"");
    assert_eq!(
        (out.as_slice(), rc),
        ("      1       1       6\n".as_bytes(), 0)
    );
}

/// A pipeline after `echo` must not be swallowed as literal arguments
/// (the #70 "silently ignored pipeline" shape).
#[test]
fn echo_led_pipeline_is_executed_not_ignored() {
    let (out, rc) = run("x=$(echo hi | wc -l); printf '%s\n' \"$x\"");
    assert_eq!((out.as_slice(), rc), (b"1\n".as_slice(), 0));
}

/// Each filter the dormant arms used to model must produce the real
/// executor's result inside `$( … )`.
#[test]
fn filter_stages_route_through_real_executor() {
    for (body, expected) in [
        ("$(printf 'b\na\n' | sort)", "a\nb\n"),
        ("$(printf 'b\na\nb\n' | sort -u)", "a\nb\n"),
        ("$(printf 'a\na\nb\n' | uniq)", "a\nb\n"),
        ("$(printf 'a\nb\n' | tail -1)", "b\n"),
        ("$(printf 'a\nb\nc\n' | head -1)", "a\n"),
        ("$(printf 'abc\n' | tr a X)", "Xbc\n"),
        ("$(printf 'abc\n' | sed s/a/X/)", "Xbc\n"),
        ("$(printf 'ax\nbx\n' | grep a)", "ax\n"),
    ] {
        let cmd = format!("x={body}; printf '%s\\n' \"$x\"");
        let (out, rc) = run(&cmd);
        assert_eq!(
            (out.as_slice(), rc),
            (expected.as_bytes(), 0),
            "{body}: pipeline substitution must run the real executor"
        );
    }
}

/// A `time`-prefixed pipeline body also routes through the real executor
/// (the removed `|` arm used to hang off the `time`-prefix shortcut).
#[test]
fn time_prefixed_pipeline_reaches_real_executor() {
    let (out, rc) = run("x=$(time printf 'b\na\n' | sort) 2>/dev/null; printf '%s' \"$x\"");
    assert_eq!((out.as_slice(), rc), (b"a\nb".as_slice(), 0));
}

/// Pipeline status semantics are the real executor's: substitution status is
/// the last stage's status (grep's 1 on no match), not 0.
#[test]
fn pipeline_substitution_status_is_last_stages() {
    let (out, rc) = run("x=$(printf 'ax\n' | grep z); printf '%s|%s\n' \"$?\" \"$x\"");
    assert_eq!((out.as_slice(), rc), (b"1|\n".as_slice(), 0));
}
