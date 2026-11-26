//! rubash#2765: heredoc body pairing can cross frame tiers when 3+
//! heredocs are pending on one physical line spanning a nested `if`
//! header.
//!
//! GNU pairs bodies purely by line order: parse.y:295 keeps one flat
//! `redir_stack` per read unit, parse.y:3116 pushes each heredoc
//! redirect as it is parsed, and gather_here_documents
//! (parse.y:3120-3130) drains the stack in push order. There are no
//! frame tiers. rubash claims pending bodies frame by frame
//! (src/parser/redirections.rs, `fill_pending_heredoc_body_*` walks
//! caller_frames before frames), so a body can be claimed by the wrong
//! frame's command when a line crosses a compound header.
//!
//! Every body below carries a distinct marker line, so any mispairing
//! shows up as wrong/missing output rows. A command with several
//! heredocs reads only the LAST one (each redirect replaces stdin),
//! which is why several GNU outputs skip early bodies.
//!
//! Expectations verified byte-for-byte against GNU Bash 5.3.20
//! script-file probes (self-compiled oracle).

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue2765-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn check(script: &str, expected_stdout: &str) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(0), "script failed: {script}\nstderr: {stderr}");
    assert_eq!(stderr, "", "script {script} wrote to stderr");
    assert_eq!(stdout, expected_stdout, "script: {script}");
}

/// Baseline shape: three separate commands, the third inside an `if`.
#[test]
fn three_commands_if_third() {
    check(
        "cat <<A; cat <<B; if true; then cat <<C; fi\nbody-a\nA\nbody-b\nB\nbody-c\nC\n",
        "body-a\nbody-b\nbody-c\n",
    );
}

/// Two heredocs on the outer command, one inside the `if`: the outer
/// cat reads only its last heredoc (B), the inner cat reads C.
#[test]
fn doubled_outer_single_inner() {
    check(
        "cat <<A <<B; if true; then cat <<C; fi\nbody-a\nA\nbody-b\nB\nbody-c\nC\n",
        "body-b\nbody-c\n",
    );
}

/// The `if` opens first: two heredocs inside the then-branch on one
/// command, one heredoc on a command after the `fi`.
#[test]
fn doubled_inside_if_single_after() {
    check(
        "if true; then cat <<A <<B; fi; cat <<C\nbody-a\nA\nbody-b\nB\nbody-c\nC\n",
        "body-b\nbody-c\n",
    );
}

/// Three heredocs on one command before an `if` with no heredoc of
/// its own: only the last body (C) reaches the cat.
#[test]
fn tripled_outer_none_inner() {
    check(
        "cat <<A <<B <<C; if true; then echo inner; fi\nbody-a\nA\nbody-b\nB\nbody-c\nC\n",
        "body-c\ninner\n",
    );
}

/// A heredoc rides the `if` CONDITION command itself.
#[test]
fn heredoc_on_if_condition() {
    check(
        "cat <<A; if cat <<B; then cat <<C; fi\nbody-a\nA\nbody-b\nB\nbody-c\nC\n",
        "body-a\nbody-b\nbody-c\n",
    );
}

/// The else tier carries two heredocs (the then tier's body is read
/// but its command never runs).
#[test]
fn doubled_in_else_branch() {
    check(
        "if false; then cat <<A; else cat <<B <<C; fi; cat <<D\nbody-a\nA\nbody-b\nB\nbody-c\nC\nbody-d\nD\n",
        "body-c\nbody-d\n",
    );
}

/// Nested `if` inside `if`, heredocs on the innermost command and
/// after the whole compound.
#[test]
fn doubled_in_nested_if() {
    check(
        "if true; then if true; then cat <<A <<B; fi; fi; cat <<C\nbody-a\nA\nbody-b\nB\nbody-c\nC\n",
        "body-b\nbody-c\n",
    );
}

/// A function-definition frame on the same line: bodies pair by line
/// order (A to the function body's cat, B to the plain cat), while
/// EXECUTION order prints B before A (the function runs last).
#[test]
fn function_frame_and_plain_command() {
    check(
        "f() { cat <<A; }; cat <<B; f\nbody-a\nA\nbody-b\nB\n",
        "body-b\nbody-a\n",
    );
}

/// A pipeline tier on the same line: the first cat's output goes down
/// the pipe (its stdin is heredoc A, the second cat's stdin is
/// heredoc B, so A's body never reaches the terminal).
#[test]
fn pipeline_two_heredocs() {
    check(
        "cat <<A | cat <<B; cat <<C\nbody-a\nA\nbody-b\nB\nbody-c\nC\n",
        "body-b\nbody-c\n",
    );
}
