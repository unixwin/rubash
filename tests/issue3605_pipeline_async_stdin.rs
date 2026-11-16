//! rubash#3605 — exec13: a pipe-fed pipeline element's async `cmd &`
//! inherits the pipe, not /dev/null.
//!
//! GNU execute_cmd.c: the forked element child ORs the async-stdin
//! verdict with its own pipe stdin (`:4570 stdin_redir |=
//! (pipe_in != NO_PIPE)`), a user subshell ASSIGNS it from its own
//! fd-0 redirects OR the feeding pipe (`:1733`), and the :826
//! control-structure assign never sees a pipe-connected element's
//! redirects (consumed by execute_in_subshell's do_redirections at
//! :1759-1764). Only reader_loop clears the flag (eval.c:181) — a
//! clone's pre-parsed element list is not a reader loop.
//!
//! Expectations byte-verified against GNU bash 5.3.20 script files.

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

fn fixture() -> String {
    let path = std::env::temp_dir().join(format!("rubash-pipestdin-{}", std::process::id()));
    std::fs::write(&path, "line1\nline2\n").unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn exec13_pipe_fed_element_async_stdin() {
    // exec13.sub's own shapes: `( )`, `(&cat)`, `{ }`, and function
    // elements of a pipe — every async `cat &` reads the pipe payload.
    let (stdout, _, _) = rubash(
        "echo x1 | ( cat & wait )\n\
         echo x1a | (:& cat & wait)\n\
         echo x3 | { cat & wait; }\n\
         x3a() { cat & wait; }\n\
         echo x3a | x3a\n\
         x3b() { echo x3b | { cat & wait; }; }\n\
         x3b\n",
    );
    assert_eq!(stdout, "x1\nx1a\nx3\nx3a\nx3b\n");
}

#[test]
fn element_own_fd0_redirect_wins_over_pipe() {
    // GNU's do_piping binds the pipe first, then the element's own
    // do_redirections replaces fd 0 with the file — the async child
    // inherits the file, not the staged payload.
    let f = fixture();
    for script in [
        format!("echo data | {{ cat & wait; }} < '{f}'"),
        format!("echo data | {{ ( cat & wait ) < '{f}'; }}"),
        format!("echo data | for i in 1; do cat & wait; done < '{f}'"),
    ] {
        let (stdout, _, _) = rubash(&script);
        assert_eq!(stdout, "line1\nline2\n", "script: {script}");
    }
}

#[test]
fn nested_subshell_clears_element_flag() {
    // GNU :1733 is an ASSIGN: a nested `( )` with no fd-0 redirect of
    // its own clears the enclosing structure's verdict — the async cat
    // inside gets /dev/null and prints nothing.
    let f = fixture();
    for script in [
        format!("{{ ( cat & wait ); }} < '{f}'"),
        "echo data | { ( cat & wait ); }".to_string(),
        format!("echo data | {{ ( cat & wait ); }} < '{f}'"),
        format!("echo data | {{ ( cat & wait ); }} > /dev/null"),
    ] {
        let (stdout, _, _) = rubash(&script);
        assert_eq!(stdout, "", "script: {script}");
    }
}

#[test]
fn element_stdin_verdict_dies_with_stage() {
    // The `|= pipe` verdict is the element child's: the next top-level
    // command's async spawn still takes the /dev/null arm, and the
    // flag never leaks into the pipeline's producer.
    let (stdout, _, _) = rubash("echo x | ( cat & wait )\nsleep 0 & wait\necho after\n");
    assert_eq!(stdout, "x\nafter\n");
}
