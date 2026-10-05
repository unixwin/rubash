//! rubash#429 — a pipeline element's OWN fd-0 file redirect must reach the
//! element's child, replacing the inherited shell stdin (stage 0) or the
//! upstream pipe payload (later stages).
//!
//! GNU: execute_cmd.c:4617 execute_simple_command applies the forked
//! element's redirections (redir.c do_redirection_internal) AFTER the
//! pipeline binds its fds, so `echo hi | cat < /dev/null` prints nothing
//! (the empty file wins over the pipe) and a stage-0 `"$BASH" --norc -i
//! < /dev/null | sed` child sees EOF on its redirected stdin and exits —
//! the bash-it iterate theme's per-prompt prompt-capture pipeline.
//!
//! Before the fix the sequential stage spawner only modeled
//! inherit-or-payload: the fd-0 file redirect was dropped, so the nested
//! interactive child inherited the live console input under ConPTY, never
//! saw EOF, and wedged the whole session (the wt91 all-themes HANG cells);
//! the piped twin fed the upstream payload (`echo hi | cat < /dev/null`
//! printed `hi`).

use std::process::Command;

fn rubash(script: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The inline-builtin twin: the empty file beats the upstream payload.
#[test]
fn dev_null_redirect_beats_upstream_payload() {
    assert_eq!(rubash("echo hi | cat < /dev/null\necho done"), "done\n");
}

/// A real file beats the upstream payload too (`seq 3 | cat < f` -> f).
#[test]
fn file_redirect_beats_upstream_payload() {
    assert_eq!(
        rubash("printf 'z\\n' > /tmp/rubash429f\nseq 3 | cat < /tmp/rubash429f"),
        "z\n"
    );
}

/// `< f` on a LATER stage also wins over the upstream pipe (GNU applies the
/// element's redirects in its forked child after the pipe binds fd 0).
#[test]
fn later_stage_file_redirect_wins() {
    assert_eq!(
        rubash("printf 'a\\n' > /tmp/rubash429g\nprintf 'up\\n' | cat | cat < /tmp/rubash429g"),
        "a\n"
    );
}

/// Heredoc winner is untouched: the stage still reads the heredoc body.
#[test]
fn heredoc_winner_still_feeds_the_stage() {
    assert_eq!(rubash("echo up | cat <<EOF\nbody\nEOF\n"), "body\n");
}

/// `<&0` keeps duping the shell's current fd 0 — the upstream pipe.
#[test]
fn fd_dup_zero_reads_the_upstream_pipe() {
    assert_eq!(rubash("printf 'p\\n' | cat <&0"), "p\n");
}

/// `/dev/stdin` keeps reading the upstream pipe (niubash#118 semantics).
#[test]
fn dev_stdin_operand_reads_the_upstream_pipe() {
    assert_eq!(rubash("printf 'q\\n' | cat < /dev/stdin"), "q\n");
}

/// Redirect-less pipelines are unchanged by the routing.
#[test]
fn plain_pipelines_unchanged() {
    assert_eq!(rubash("seq 3 | tail -1"), "3\n");
    assert_eq!(rubash("seq 3 | head -1"), "1\n");
}

/// The wt91 wedge shape end-to-end, non-interactive twin: the nested rubash
/// child in a pipeline must exit on its redirected /dev/null stdin (under a
/// tty the pre-fix child blocked on the inherited console forever). The
/// child's prompt/exit output proves it ran; the pipeline completing proves
/// the session was not wedged.
#[test]
fn nested_interactive_child_in_pipeline_exits_on_dev_null_stdin() {
    let script = format!(
        "out=$({} --norc -i < /dev/null 2>&1 | cat); echo \"AFTER-[$out]\"",
        env!("CARGO_BIN_EXE_rubash")
    );
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(&script)
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        stdout.starts_with("AFTER-["),
        "nested interactive pipeline wedged the session; stdout: {stdout:?}"
    );
}
