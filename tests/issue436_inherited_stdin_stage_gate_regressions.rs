//! rubash#436 — the inherited-process-stdin fallback must not consume the
//! shell's own input while a captured pipeline-stage stdin is armed.
//!
//! GNU: execute_cmd.c execute_pipeline dup2s the pipe over fd 0 for every
//! pipeline element after the first (execute_cmd.c:2702+), so an element
//! reading EOF-on-empty-pipe never touches the shell's stdin;
//! subst.c:7143 command_substitute keeps the shell's stdin only where no
//! pipe was dup2'd. `niu -i` arms INHERIT_PROCESS_STDIN for the whole
//! session, so the `sed` shim's read-inherited fallback drained the
//! interactive driver's pending input lines whenever a stage's captured
//! pipe payload was EMPTY (the argcomplete nox.bash loader wedge:
//! `eval -- "$( pathcmd=$(type -P -- "$1" 2>/dev/null | command sed
//! 's,/[^/]*$,,') ... )"` — deterministic echo-only session); on a live
//! console the read blocked forever instead.
//!
//! The fix: `read_inherited_process_stdin_to_string` declines while
//! FUNCTION_STDIN is armed — an armed buffer is this command's fd 0
//! (the pipe), empty meaning EOF, never an invitation to read the
//! process stdin.

use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

fn rubash_stdin(script: &str, feed: &str) -> String {
    use std::io::Write;
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    {
        let mut stdin = child.stdin.take().expect("stdin");
        stdin.write_all(feed.as_bytes()).expect("write feed");
    }
    let output = child.wait_with_output().expect("reap rubash");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The nox.bash shape: an empty-output nested pipeline inside an outer
/// command substitution must leave the shell's stdin for later readers.
/// Before the fix the armed-empty stage stdin fell through to the
/// inherited-stdin drain, which consumed `LEFTOVER\n` (the interactive
/// driver's next input lines under `niu -i`).
#[test]
fn empty_stage_pipe_keeps_shell_stdin_for_later_readers() {
    let out = rubash_stdin(
        "__RUBASH_INHERIT_PROCESS_STDIN=1\nq=$(true 2>/dev/null | command sed 's,/[^/]*$,,')\nrest=$(cat)\necho \"rest=[$rest]\"",
        "LEFTOVER\n",
    );
    assert_eq!(out, "rest=[LEFTOVER]\n");
}

/// A non-empty stage payload is still consumed as the element's stdin
/// (the pipe content, not the shell's stdin).
#[test]
fn nonempty_stage_pipe_still_feeds_the_element() {
    let out = rubash_stdin(
        "__RUBASH_INHERIT_PROCESS_STDIN=1\necho MARK-A\nd=$(echo dir1 | command sed 's,dir1,dir2,')\nrest=$(cat)\necho \"d=[$d] rest=[$rest]\"",
        "LEFTOVER\n",
    );
    assert_eq!(out, "MARK-A\nd=[dir2] rest=[LEFTOVER]\n");
}

/// The nox.bash loader verbatim shape (eval -- of the nested-comsub text)
/// under the session-wide inherit flag: the trailing script input stays
/// readable afterwards.
#[test]
fn argcomplete_loader_shape_leaves_stdin_intact() {
    let script = "__RUBASH_INHERIT_PROCESS_STDIN=1\n\
                  eval -- \"$(\n    pathcmd=$(type -P -- \"$1\" 2>/dev/null | command sed 's,/[^/]*$,,')\n    [[ $pathcmd ]] && PATH=$pathcmd${PATH:+:$PATH}\n)\"\n\
                  rest=$(cat)\n\
                  echo \"rest=[$rest]\"";
    let out = rubash_stdin(script, "LEFTOVER\n");
    assert_eq!(out, "rest=[LEFTOVER]\n");
}

/// An element after an empty stage still reads its own upstream payload
/// (the gate must not switch the pipe off, only the process-stdin drain).
#[test]
fn downstream_stage_still_reads_its_own_pipe() {
    let out = rubash_stdin(
        "__RUBASH_INHERIT_PROCESS_STDIN=1\nout=$(printf 'up\\n' | true | command sed 's,up,down,')\necho \"out=[$out]\"",
        "",
    );
    // `true` contributes nothing; sed reads the (empty) upstream pipe and
    // prints nothing — GNU parity for `printf 'up\n' | true | sed`.
    assert_eq!(out, "out=[]\n");
}

/// The interactive wedge twin: an empty-pipe element under the session
/// inherit flag must not block reading a live console. Fed an open pipe
/// that never produces data, the command must still finish (GNU: the
/// element reads the drained pipe and exits immediately).
#[test]
fn empty_stage_does_not_block_on_the_session_stdin() {
    let (tx, rx) = mpsc::channel();
    let script =
        "__RUBASH_INHERIT_PROCESS_STDIN=1\nq=$(true | command sed 's,x,y,')\necho done".to_string();
    std::thread::spawn(move || {
        // stdin left open but silent: a console-shaped stdin. The old
        // code blocked in read_to_string here forever.
        let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg("-c")
            .arg(&script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn rubash");
        // Hold the stdin pipe open (never write, never close).
        let _stdin = child.stdin.take();
        let out = child.wait_with_output().expect("reap rubash");
        let _ = tx.send(String::from_utf8_lossy(&out.stdout).into_owned());
    });
    let out = rx
        .recv_timeout(Duration::from_secs(30))
        .expect("empty-pipe element must not block on session stdin");
    assert_eq!(out, "done\n");
}
