//! rubash#2174 — builtin output routed to the real process stdout
//! lands in the Rust std stdout handle's internal line buffer and
//! stays pending when it carries no newline, while an external
//! command's writes go straight to fd 1. Without a flush at the
//! command boundary the next external writer (spawn, pipeline stage,
//! process-substitution consumer, background child) overtakes the
//! pending bytes: `printf 'A'; /usr/bin/printf 'B'; printf 'C'; echo`
//! printed `BAC` where GNU prints `ABC`.
//!
//! GNU's rule is structural: execute_cmd.c:5648
//! (execute_builtin_or_function) runs `fflush (stdout)` after EVERY
//! builtin or function executed in the current shell context, before
//! undoing the command's redirections — at any command boundary the
//! shell's stdio buffer is empty, so program order on a shared fd is
//! preserved by construction. rubash mirrors that invariant with a
//! flush of the process std stdout handle at the completion of every
//! simple command in execute_materialized_command. The buffer is the
//! process stdio buffer, not a printf-private one: `echo -n` (whose
//! writes reach the same handle) diverged identically on master.
//!
//! Expectations measured against GNU bash 5.3.20 (script files,
//! stdout captured, 2026-10-30): every shape below matches GNU
//! byte-for-byte. Unix-only: the shapes spell POSIX paths and use
//! /usr/bin/printf + od as the foreign writers on the shared fd.

#![cfg(unix)]

use std::process::Command;

fn run_script(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash2174-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    let case = dir.join("case.sh");
    std::fs::write(&case, script).expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg(&case)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn normalized(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn issue2174_printf_external_interleave_keeps_program_order() {
    let (stdout, stderr, code) = run_script(
        "minimal",
        "printf 'A'; /usr/bin/printf 'B'; printf 'C'; echo\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "ABC\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2174_echo_dash_n_shares_the_same_stdio_buffer() {
    // Not a printf-only defect: echo -n reaches the same process
    // stdio buffer and was deferred past the external writer too.
    let (stdout, stderr, code) = run_script(
        "echon",
        "echo -n 'A'; /usr/bin/printf 'B'; echo -n 'C'; echo\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "ABC\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2174_pipeline_shapes_keep_program_order() {
    let (stdout, stderr, code) = run_script(
        "pipeext",
        "printf 'A'; /usr/bin/printf 'B' | cat; printf 'C'; echo\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "ABC\n");
    assert_eq!(stderr, "");

    // Builtin as pipeline producer: the pending label printed before
    // the pipeline must precede the last stage's output on fd 1.
    let (stdout, stderr, code) = run_script(
        "pipeprod",
        "printf 'X'; printf 'Y' | od -An -c; printf 'Z'; echo\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "X   Y\nZ\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2174_command_substitution_label_pipeline_grouping() {
    // The #2163 victim shape: label printfs and per-byte od pipeline
    // outputs must interleave pairwise, not group by producer.
    let (stdout, stderr, code) = run_script(
        "c0group",
        "x=$(printf '\\024'); printf '14 '; printf '%s' \"$x\" | od -An -tx1; \
         x=$(printf '\\025'); printf '15 '; printf '%s' \"$x\" | od -An -tx1; \
         x=$(printf '\\032'); printf '1a '; printf '%s' \"$x\" | od -An -tx1; \
         x=$(printf '\\037'); printf '1f '; printf '%s' \"$x\" | od -An -tx1\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(normalized(&stdout), "14 14 15 15 1a 1a 1f 1f");
    assert_eq!(stderr, "");
}

#[test]
fn issue2174_dollar_and_backtick_substitution_forms() {
    let (stdout, stderr, code) = run_script(
        "dollar",
        "v=$(printf 'A'); printf '%s' \"$v\"; /usr/bin/printf 'B'; echo\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "AB\n");
    assert_eq!(stderr, "");

    let (stdout, stderr, code) = run_script(
        "backtick",
        "v=`printf 'A'`; printf '%s' \"$v\"; /usr/bin/printf 'B'; echo\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "AB\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2174_mixed_builtin_external_with_redirects() {
    // A redirected external neither flushes nor is flushed by the
    // pending stdout bytes; the un-redirected external after it must
    // still come after both pending builtins.
    let (stdout, stderr, code) = run_script(
        "redirfile",
        "printf 'A'; /usr/bin/printf 'B' > out.txt; printf 'C'; /usr/bin/printf 'D'; echo; cat out.txt\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "ACD\nB");
    assert_eq!(stderr, "");

    // stdout and stderr merged by the group redirect share one fd:
    // the stderr-bound builtin is ordered with the external writer.
    let (stdout, stderr, code) = run_script(
        "group21",
        "{ printf 'A' >&2; /usr/bin/printf 'B'; printf 'C'; } 2>&1; echo\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "ABC\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2174_background_and_function_shapes_keep_program_order() {
    let (stdout, stderr, code) = run_script(
        "bg",
        "printf 'D'; /usr/bin/printf 'E' & wait; printf 'F'; echo\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "DEF\n");
    assert_eq!(stderr, "");

    let (stdout, stderr, code) = run_script(
        "func",
        "f() { printf 'G'; /usr/bin/printf 'H'; }; f; echo\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "GH\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2174_procsub_consumer_keeps_program_order() {
    // The process-substitution consumer is a foreign writer on fd 1;
    // the label printed before its creating command must precede the
    // consumer's output, for builtin-fed and external-fed forms alike.
    let (stdout, stderr, code) = run_script(
        "procsubin",
        "printf 'L'; printf 'M' > >(od -An -c); sleep 0.2; printf 'N'; echo\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "L   M\nN\n");
    assert_eq!(stderr, "");

    let (stdout, stderr, code) = run_script(
        "procsubext",
        "printf 'L'; /usr/bin/printf 'M' > >(od -An -c); sleep 0.2; printf 'N'; echo\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "L   M\nN\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2174_background_subshell_keeps_program_order() {
    let (stdout, stderr, code) = run_script(
        "bgsub",
        "printf 'A'; (printf 'B') & wait; printf 'C'; echo\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "ABC\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2174_loop_body_interleave_keeps_program_order() {
    let (stdout, stderr, code) = run_script(
        "loop",
        "i=0; while [ $i -lt 2 ]; do printf 'x'; /usr/bin/printf 'y'; i=$((i+1)); done; echo\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "xyxy\n");
    assert_eq!(stderr, "");
}
