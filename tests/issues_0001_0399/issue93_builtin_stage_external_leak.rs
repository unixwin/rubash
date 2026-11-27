//! Issue unixwin/niubash#93 regression: a pipeline stage headed by a builtin
//! that transitively spawns an external command (`command find`, `eval "find
//! ..."`, `env find`) must pipe that child's stdout into the stage capture.
//! The stage wrapper only armed the thread-local builtin capture;
//! execute_external gated its child-stdout pipe on the Executor
//! `stdout_capture` field alone, so the child inherited the process stdout —
//! the producer output leaked to real stdout and the downstream stage
//! consumed an empty stream (`x=$(command find . | wc -l)` printed the
//! listing and assigned x=0).
//! GNU baseline: every pipeline element's fd 1 is the pipe (execute_cmd.c
//! execute_pipeline + redir.c), regardless of which builtin dispatches the
//! command inside the element's subshell.

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

/// Creates a per-test temp directory holding a three-line fixture and returns
/// the `cd`-prefixed script. The directory is removed on drop.
struct Fixture {
    dir: std::path::PathBuf,
}

impl Fixture {
    fn script(&self, body: &str) -> String {
        format!(
            "cd '{}' || exit 1\n{}\n",
            self.dir.to_string_lossy().replace('\\', "/"),
            body
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn fixture(tag: &str) -> Fixture {
    let dir = std::env::temp_dir().join(format!("rubash-issue93-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue93 fixture dir");
    std::fs::write(dir.join("lines.txt"), "alpha\nbeta\ngamma\n").expect("write fixture");
    Fixture { dir }
}

#[test]
fn command_prefixed_external_first_stage_feeds_the_pipe() {
    // `command cat` resolves through PATH to the real external cat; its
    // stdout must reach `wc -l`, not the terminal.
    let fixture = fixture("command");
    let (stdout, _, code) =
        rubash(&fixture.script("x=$(command cat lines.txt | wc -l); printf '<%s>\\n' \"$x\""));
    assert_eq!(stdout, "<3>\n");
    assert_eq!(code, Some(0));
}

#[test]
fn eval_external_first_stage_feeds_the_pipe() {
    let fixture = fixture("eval");
    let (stdout, _, _) =
        rubash(&fixture.script("x=$(eval \"cat lines.txt\" | wc -l); printf '<%s>\\n' \"$x\""));
    assert_eq!(stdout, "<3>\n");
}

#[test]
fn env_external_first_stage_feeds_the_pipe() {
    let fixture = fixture("env");
    let (stdout, _, _) =
        rubash(&fixture.script("x=$(env cat lines.txt | wc -l); printf '<%s>\\n' \"$x\""));
    assert_eq!(stdout, "<3>\n");
}

#[test]
fn command_prefixed_stage_output_does_not_leak_to_stdout() {
    // Without a pipe consumer the same mechanism leaks: the captured
    // assignment must carry the listing and nothing may reach real stdout.
    let fixture = fixture("noleak");
    let (stdout, _, _) =
        rubash(&fixture.script("x=$(command cat lines.txt); printf '[%s]\\n' \"$x\""));
    assert_eq!(stdout, "[alpha\nbeta\ngamma]\n");
}

#[test]
fn eval_procsub_pipeline_does_not_leak_and_feeds_reader() {
    // `< <(eval "cat f" | sort)`: the eval'd external ran with inherited
    // stdout, so the 3 lines escaped to the terminal and sort saw EOF.
    let fixture = fixture("procsub");
    let (stdout, _, _) = rubash(&fixture.script(
        "c=0; while IFS= read -r _; do c=$((c+1)); done < <(eval \"cat lines.txt\" | sort); printf 'c=%s\\n' \"$c\"",
    ));
    assert_eq!(stdout, "c=3\n");
}

#[test]
fn procsub_plain_pipeline_still_counts_lines() {
    let fixture = fixture("plain");
    let (stdout, _, _) = rubash(&fixture.script(
        "c=0; while IFS= read -r _; do c=$((c+1)); done < <(cat lines.txt); printf 'c=%s\\n' \"$c\"",
    ));
    assert_eq!(stdout, "c=3\n");
}

#[test]
fn builtin_wrapped_external_keeps_fd1_write_order() {
    // Mixed builtin/external writes inside one eval'd stage body must keep
    // fd-1 order through the single thread-local capture.
    let fixture = fixture("order");
    let (stdout, _, _) =
        rubash(&fixture.script("eval \"printf 'first\\\\n'; cat lines.txt | wc -l\" | cat"));
    assert_eq!(stdout, "first\n3\n");
}
